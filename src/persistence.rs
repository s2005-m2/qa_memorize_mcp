use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};

use crate::embedding::Embedder;
use crate::models::*;
use crate::storage::Storage;

const JSON_FILENAME: &str = "memorize_data.json";
const SIMILAR_THRESHOLD: f32 = 0.15;

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
    let mut qa_records = storage.dump_qa().await?;
    let mut knowledge = storage.dump_knowledge().await?;

    let now = chrono_now();
    for r in &mut qa_records {
        if r.created_at.is_none() {
            r.created_at = Some(now.clone());
        }
    }
    for r in &mut knowledge {
        if r.created_at.is_none() {
            r.created_at = Some(now.clone());
        }
    }

    let snapshot = MemorizeSnapshot {
        version: 1,
        exported_at: now,
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

async fn import_one_shared(
    storage: &Storage,
    embedder: &Embedder,
    path: &Path,
    errors: &mut Vec<String>,
) -> Result<()> {
    let json_str = std::fs::read_to_string(path)?;
    let snapshot: MemorizeSnapshot = serde_json::from_str(&json_str)?;
    let fname = path.display().to_string();

    tracing::info!(
        "Importing {} ({} topics, {} QA, {} knowledge)",
        fname, snapshot.topics.len(), snapshot.qa_records.len(), snapshot.knowledge.len()
    );

    for entry in &snapshot.topics {
        let vec = embedder.embed(&entry.topic_name)?;
        if storage.find_similar_topic(&vec, DEFAULT_TOPIC_THRESHOLD).await?.is_none() {
            storage.create_topic(&entry.topic_name, &vec).await?;
        }
    }

    let fallback_time = &snapshot.exported_at;

    for entry in &snapshot.qa_records {
        if let Err(e) = merge_qa(storage, embedder, entry, fallback_time).await {
            errors.push(format!("[{}] QA '{}': {}", fname, entry.question, e));
        }
    }

    for entry in &snapshot.knowledge {
        if let Err(e) = merge_knowledge_entry(storage, embedder, entry, fallback_time).await {
            let preview = &entry.knowledge_text[..entry.knowledge_text.len().min(50)];
            errors.push(format!("[{}] Knowledge '{}': {}", fname, preview, e));
        }
    }

    Ok(())
}

// ── Merge Helpers ──

async fn merge_qa(
    storage: &Storage,
    embedder: &Embedder,
    entry: &QaEntry,
    fallback_time: &str,
) -> Result<()> {
    let vec = embedder.embed(&entry.question)?;
    let existing = storage.find_nearest_qa_global(&vec).await?;

    if let Some(ref record) = existing {
        if record.score <= SIMILAR_THRESHOLD {
            let incoming_time = entry.created_at.as_deref().unwrap_or(fallback_time);
            let all_qa = storage.dump_qa().await?;
            let existing_time = all_qa.iter()
                .find(|r| r.question == record.question && r.topic == record.topic)
                .and_then(|r| r.created_at.as_deref())
                .unwrap_or("");

            if incoming_time > existing_time {
                storage.delete_qa(&record.question, &record.topic).await?;
                let topic = resolve_topic(storage, embedder, &entry.topic).await?;
                storage.insert_qa_with_merged(
                    &entry.question, &entry.answer, &topic, entry.merged, &vec,
                ).await?;
            }
            return Ok(());
        }
    }

    let topic = resolve_topic(storage, embedder, &entry.topic).await?;
    storage.insert_qa_with_merged(
        &entry.question, &entry.answer, &topic, entry.merged, &vec,
    ).await?;
    Ok(())
}

async fn merge_knowledge_entry(
    storage: &Storage,
    embedder: &Embedder,
    entry: &KnowledgeEntry,
    fallback_time: &str,
) -> Result<()> {
    let vec = embedder.embed(&entry.knowledge_text)?;
    let existing = storage.find_nearest_knowledge_global(&vec).await?;

    if let Some(ref record) = existing {
        if record.score <= SIMILAR_THRESHOLD {
            let incoming_time = entry.created_at.as_deref().unwrap_or(fallback_time);
            let all_knowledge = storage.dump_knowledge().await?;
            let existing_time = all_knowledge.iter()
                .find(|r| r.knowledge_text == record.knowledge_text && r.topic == record.topic)
                .and_then(|r| r.created_at.as_deref())
                .unwrap_or("");

            if incoming_time > existing_time {
                storage.delete_knowledge(&record.knowledge_text, &record.topic).await?;
                let topic = resolve_topic(storage, embedder, &entry.topic).await?;
                storage.insert_knowledge(
                    &entry.knowledge_text, &topic, &entry.source_questions, &vec,
                ).await?;
            }
            return Ok(());
        }
    }

    let topic = resolve_topic(storage, embedder, &entry.topic).await?;
    storage.insert_knowledge(
        &entry.knowledge_text, &topic, &entry.source_questions, &vec,
    ).await?;
    Ok(())
}

async fn resolve_topic(
    storage: &Storage,
    embedder: &Embedder,
    topic_name: &str,
) -> Result<String> {
    let vec = embedder.embed(topic_name)?;
    if let Some(existing) = storage.find_similar_topic(&vec, DEFAULT_TOPIC_THRESHOLD).await? {
        return Ok(existing);
    }
    storage.create_topic(topic_name, &vec).await?;
    Ok(topic_name.to_string())
}

// ── Sync on Startup ──

pub async fn sync_on_startup(
    storage: &Storage,
    embedder: &Embedder,
    data_dir: &Path,
) -> Result<()> {
    let path = json_path(data_dir);
    if !path.exists() {
        return Ok(());
    }

    let json_str = std::fs::read_to_string(&path)
        .map_err(|e| anyhow!("Failed to read {}: {}", path.display(), e))?;
    let snapshot: MemorizeSnapshot = serde_json::from_str(&json_str)
        .map_err(|e| anyhow!("Failed to parse {}: {}", path.display(), e))?;

    tracing::info!(
        "Loaded JSON snapshot (v{}, exported_at: {}) with {} topics, {} QA, {} knowledge",
        snapshot.version, snapshot.exported_at,
        snapshot.topics.len(), snapshot.qa_records.len(), snapshot.knowledge.len()
    );

    let mut json_to_db_topics = 0u32;
    let mut json_to_db_qa = 0u32;
    let mut json_to_db_knowledge = 0u32;

    for entry in &snapshot.topics {
        if storage.has_topic(&entry.topic_name).await? {
            continue;
        }
        let vector = embedder.embed(&entry.topic_name)?;
        storage.create_topic(&entry.topic_name, &vector).await?;
        json_to_db_topics += 1;
    }

    for entry in &snapshot.qa_records {
        if storage.has_qa(&entry.question, &entry.topic).await? {
            continue;
        }
        let vector = embedder.embed(&entry.question)?;
        storage
            .insert_qa_with_merged(&entry.question, &entry.answer, &entry.topic, entry.merged, &vector)
            .await?;
        json_to_db_qa += 1;
    }

    for entry in &snapshot.knowledge {
        if storage.has_knowledge(&entry.knowledge_text, &entry.topic).await? {
            continue;
        }
        let vector = embedder.embed(&entry.knowledge_text)?;
        storage
            .insert_knowledge(&entry.knowledge_text, &entry.topic, &entry.source_questions, &vector)
            .await?;
        json_to_db_knowledge += 1;
    }

    if json_to_db_topics + json_to_db_qa + json_to_db_knowledge > 0 {
        tracing::info!(
            "JSON → LanceDB: +{} topics, +{} QA, +{} knowledge",
            json_to_db_topics, json_to_db_qa, json_to_db_knowledge
        );
    }

    let db_topics = storage.dump_topics().await?;
    let db_qa = storage.dump_qa().await?;
    let db_knowledge = storage.dump_knowledge().await?;

    let json_topic_names: HashSet<&str> = snapshot.topics.iter().map(|t| t.topic_name.as_str()).collect();
    let json_qa_keys: HashSet<(&str, &str)> = snapshot.qa_records.iter()
        .map(|r| (r.question.as_str(), r.topic.as_str())).collect();
    let json_knowledge_keys: HashSet<(&str, &str)> = snapshot.knowledge.iter()
        .map(|r| (r.knowledge_text.as_str(), r.topic.as_str())).collect();

    let db_has_extra = db_topics.iter().any(|t| !json_topic_names.contains(t.topic_name.as_str()))
        || db_qa.iter().any(|r| !json_qa_keys.contains(&(r.question.as_str(), r.topic.as_str())))
        || db_knowledge.iter().any(|r| !json_knowledge_keys.contains(&(r.knowledge_text.as_str(), r.topic.as_str())));

    if db_has_extra {
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

// ── Time Helpers ──

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

// ── Import Shared ──

pub async fn import_shared(
    storage: &Storage,
    embedder: &Embedder,
    data_dir: &Path,
) -> Result<()> {
    let shared_files: Vec<_> = std::fs::read_dir(data_dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.extension().map_or(false, |ext| ext == "json")
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .map_or(false, |n| n.ends_with("_shared.json"))
        })
        .collect();

    if shared_files.is_empty() {
        return Ok(());
    }

    tracing::info!("Found {} shared file(s) to import", shared_files.len());
    let mut errors: Vec<String> = Vec::new();

    for file_path in &shared_files {
        let fname = file_path.display().to_string();
        match import_one_shared(storage, embedder, file_path, &mut errors).await {
            Ok(()) => {
                if let Err(e) = std::fs::remove_file(file_path) {
                    errors.push(format!("[{}] Failed to delete after import: {}", fname, e));
                } else {
                    tracing::info!("Imported and deleted {}", fname);
                }
            }
            Err(e) => {
                errors.push(format!("[{}] Import failed: {}", fname, e));
            }
        }
    }

    if !errors.is_empty() {
        let log_path = data_dir.join("error.log");
        let log_content = errors.join("\n");
        if let Err(e) = std::fs::write(&log_path, &log_content) {
            tracing::error!("Failed to write error.log: {}", e);
        } else {
            tracing::warn!("Import errors written to {}", log_path.display());
        }
    }

    Ok(())
}
