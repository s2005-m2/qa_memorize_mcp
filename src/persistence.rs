use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};

use crate::embedding::Embedder;
use crate::models::*;
use crate::storage::Storage;

const JSON_FILENAME: &str = "memorize_data.json";

pub fn default_data_dir() -> Result<PathBuf> {
    let home = if cfg!(target_os = "windows") {
        std::env::var("USERPROFILE")
            .or_else(|_| std::env::var("HOME"))
            .map_err(|_| anyhow!("Cannot determine home directory (USERPROFILE/HOME not set)"))?
    } else {
        std::env::var("HOME")
            .map_err(|_| anyhow!("Cannot determine home directory (HOME not set)"))?
    };
    Ok(PathBuf::from(home).join(".memorize-mcp"))
}

pub fn json_path(data_dir: &Path) -> PathBuf {
    data_dir.join(JSON_FILENAME)
}

pub async fn export_json(storage: &Storage, data_dir: &Path) -> Result<()> {
    let topics = storage.dump_topics().await?;
    let qa_records = storage.dump_qa().await?;
    let knowledge = storage.dump_knowledge().await?;

    let snapshot = MemorizeSnapshot {
        version: 1,
        exported_at: chrono_now(),
        topics,
        qa_records,
        knowledge,
    };

    std::fs::create_dir_all(data_dir)?;
    let path = json_path(data_dir);
    let json = serde_json::to_string_pretty(&snapshot)
        .map_err(|e| anyhow!("Failed to serialize snapshot: {}", e))?;
    std::fs::write(&path, json)
        .map_err(|e| anyhow!("Failed to write {}: {}", path.display(), e))?;

    tracing::info!(
        "Exported {} topics, {} QA records, {} knowledge entries to {}",
        snapshot.topics.len(),
        snapshot.qa_records.len(),
        snapshot.knowledge.len(),
        path.display()
    );
    Ok(())
}

pub async fn sync_on_startup(
    storage: &Storage,
    embedder: &Embedder,
    data_dir: &Path,
) -> Result<()> {
    let path = json_path(data_dir);
    if !path.exists() {
        tracing::info!("No existing JSON snapshot at {}, skipping sync", path.display());
        return Ok(());
    }

    let json_str = std::fs::read_to_string(&path)
        .map_err(|e| anyhow!("Failed to read {}: {}", path.display(), e))?;
    let snapshot: MemorizeSnapshot = serde_json::from_str(&json_str)
        .map_err(|e| anyhow!("Failed to parse {}: {}", path.display(), e))?;

    tracing::info!(
        "Loaded JSON snapshot (v{}, exported_at: {}) with {} topics, {} QA, {} knowledge",
        snapshot.version,
        snapshot.exported_at,
        snapshot.topics.len(),
        snapshot.qa_records.len(),
        snapshot.knowledge.len()
    );

    // ── JSON → LanceDB: insert records that exist in JSON but not in LanceDB ──

    let mut json_to_db_topics = 0u32;
    let mut json_to_db_qa = 0u32;
    let mut json_to_db_knowledge = 0u32;

    for entry in &snapshot.topics {
        if storage.has_topic(&entry.topic_name).await? {
            continue;
        }
        let vector = match &entry.vector {
            Some(v) if v.len() == VECTOR_DIM as usize => v.clone(),
            _ => embedder.embed(&entry.topic_name)?,
        };
        storage.create_topic(&entry.topic_name, &vector).await?;
        json_to_db_topics += 1;
    }

    for entry in &snapshot.qa_records {
        if storage.has_qa(&entry.question, &entry.topic).await? {
            continue;
        }
        let vector = match &entry.vector {
            Some(v) if v.len() == VECTOR_DIM as usize => v.clone(),
            _ => embedder.embed(&entry.question)?,
        };
        storage
            .insert_qa_with_merged(&entry.question, &entry.answer, &entry.topic, entry.merged, &vector)
            .await?;
        json_to_db_qa += 1;
    }

    for entry in &snapshot.knowledge {
        if storage
            .has_knowledge(&entry.knowledge_text, &entry.topic)
            .await?
        {
            continue;
        }
        let vector = match &entry.vector {
            Some(v) if v.len() == VECTOR_DIM as usize => v.clone(),
            _ => embedder.embed(&entry.knowledge_text)?,
        };
        storage
            .insert_knowledge(&entry.knowledge_text, &entry.topic, &entry.source_questions, &vector)
            .await?;
        json_to_db_knowledge += 1;
    }

    if json_to_db_topics + json_to_db_qa + json_to_db_knowledge > 0 {
        tracing::info!(
            "JSON → LanceDB: +{} topics, +{} QA, +{} knowledge",
            json_to_db_topics,
            json_to_db_qa,
            json_to_db_knowledge
        );
    }

    // ── LanceDB → JSON: find records in DB that are missing from JSON, then re-export ──

    let db_topics = storage.dump_topics().await?;
    let db_qa = storage.dump_qa().await?;
    let db_knowledge = storage.dump_knowledge().await?;

    let json_topic_names: HashSet<&str> = snapshot.topics.iter().map(|t| t.topic_name.as_str()).collect();
    let json_qa_keys: HashSet<(&str, &str)> = snapshot
        .qa_records
        .iter()
        .map(|r| (r.question.as_str(), r.topic.as_str()))
        .collect();
    let json_knowledge_keys: HashSet<(&str, &str)> = snapshot
        .knowledge
        .iter()
        .map(|r| (r.knowledge_text.as_str(), r.topic.as_str()))
        .collect();

    let db_has_extra_topics = db_topics
        .iter()
        .any(|t| !json_topic_names.contains(t.topic_name.as_str()));
    let db_has_extra_qa = db_qa
        .iter()
        .any(|r| !json_qa_keys.contains(&(r.question.as_str(), r.topic.as_str())));
    let db_has_extra_knowledge = db_knowledge
        .iter()
        .any(|r| !json_knowledge_keys.contains(&(r.knowledge_text.as_str(), r.topic.as_str())));

    if db_has_extra_topics || db_has_extra_qa || db_has_extra_knowledge {
        tracing::info!("LanceDB has records not in JSON, re-exporting snapshot");
        let updated = MemorizeSnapshot {
            version: 1,
            exported_at: chrono_now(),
            topics: db_topics,
            qa_records: db_qa,
            knowledge: db_knowledge,
        };
        let json = serde_json::to_string_pretty(&updated)
            .map_err(|e| anyhow!("Failed to serialize snapshot: {}", e))?;
        std::fs::write(&path, json)
            .map_err(|e| anyhow!("Failed to write {}: {}", path.display(), e))?;
    }

    Ok(())
}

fn chrono_now() -> String {
    use std::time::SystemTime;
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs();
    // ISO 8601 UTC without pulling in chrono crate
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    // Days since 1970-01-01
    let mut y = 1970i64;
    let mut remaining_days = days as i64;
    loop {
        let days_in_year = if is_leap(y) { 366 } else { 365 };
        if remaining_days < days_in_year {
            break;
        }
        remaining_days -= days_in_year;
        y += 1;
    }
    let month_days = if is_leap(y) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };
    let mut m = 0usize;
    for (i, &md) in month_days.iter().enumerate() {
        if remaining_days < md as i64 {
            m = i;
            break;
        }
        remaining_days -= md as i64;
    }
    let d = remaining_days + 1;
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        y,
        m + 1,
        d,
        hours,
        minutes,
        seconds
    )
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}
