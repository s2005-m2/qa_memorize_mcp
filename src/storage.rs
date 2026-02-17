use anyhow::{anyhow, Result};
use arrow_array::{
    builder::{ListBuilder, StringBuilder},
    types::Float32Type,
    Array, BooleanArray, FixedSizeListArray, Float32Array, ListArray, RecordBatch,
    RecordBatchIterator, StringArray,
};
use arrow_schema::{DataType, Field, Schema};
use futures::TryStreamExt;
use lancedb::query::{ExecutableQuery, QueryBase};
use std::sync::Arc;

use crate::models::{KnowledgeEntry, KnowledgeRecord, QaEntry, QaRecord, TopicEntry, VECTOR_DIM};

fn topics_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("topic_name", DataType::Utf8, false),
        Field::new(
            "vector",
            DataType::FixedSizeList(
                Arc::new(Field::new("item", DataType::Float32, true)),
                VECTOR_DIM,
            ),
            false,
        ),
    ]))
}

fn qa_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("question", DataType::Utf8, false),
        Field::new("answer", DataType::Utf8, false),
        Field::new("topic", DataType::Utf8, false),
        Field::new("merged", DataType::Boolean, false),
        Field::new(
            "vector",
            DataType::FixedSizeList(
                Arc::new(Field::new("item", DataType::Float32, true)),
                VECTOR_DIM,
            ),
            false,
        ),
    ]))
}

fn knowledge_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("knowledge_text", DataType::Utf8, false),
        Field::new("topic", DataType::Utf8, false),
        Field::new(
            "source_questions",
            DataType::List(Arc::new(Field::new("item", DataType::Utf8, true))),
            true,
        ),
        Field::new(
            "vector",
            DataType::FixedSizeList(
                Arc::new(Field::new("item", DataType::Float32, true)),
                VECTOR_DIM,
            ),
            false,
        ),
    ]))
}

fn make_vector_array(vector: &[f32]) -> Arc<FixedSizeListArray> {
    Arc::new(
        FixedSizeListArray::from_iter_primitive::<Float32Type, _, _>(
            vec![Some(vector.iter().map(|v| Some(*v)).collect::<Vec<_>>())],
            VECTOR_DIM,
        ),
    )
}

pub struct Storage {
    #[allow(dead_code)]
    db: lancedb::Connection,
    topics: lancedb::Table,
    qa_records: lancedb::Table,
    knowledge: lancedb::Table,
}

impl Storage {
    pub async fn open(db_path: &str) -> Result<Self> {
        let db = lancedb::connect(db_path).execute().await?;
        let table_names = db.table_names().execute().await?;

        let topics = if table_names.contains(&"topics".to_string()) {
            db.open_table("topics").execute().await?
        } else {
            db.create_empty_table("topics", topics_schema())
                .execute()
                .await?
        };

        let qa_records = if table_names.contains(&"qa_records".to_string()) {
            db.open_table("qa_records").execute().await?
        } else {
            db.create_empty_table("qa_records", qa_schema())
                .execute()
                .await?
        };

        let knowledge = if table_names.contains(&"knowledge".to_string()) {
            db.open_table("knowledge").execute().await?
        } else {
            db.create_empty_table("knowledge", knowledge_schema())
                .execute()
                .await?
        };

        Ok(Self {
            db,
            topics,
            qa_records,
            knowledge,
        })
    }

    pub async fn create_topic(&self, name: &str, vector: &[f32]) -> Result<()> {
        let schema = topics_schema();
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(StringArray::from(vec![name])),
                make_vector_array(vector),
            ],
        )?;
        let batches = RecordBatchIterator::new(vec![Ok(batch)], schema);
        self.topics.add(Box::new(batches)).execute().await?;
        Ok(())
    }

    pub async fn find_similar_topic(
        &self,
        vector: &[f32],
        threshold: f32,
    ) -> Result<Option<String>> {
        let batches: Vec<RecordBatch> = self
            .topics
            .query()
            .nearest_to(vector)?
            .limit(1)
            .execute()
            .await?
            .try_collect()
            .await?;

        if batches.is_empty() || batches[0].num_rows() == 0 {
            return Ok(None);
        }

        let batch = &batches[0];
        let distances = batch
            .column_by_name("_distance")
            .ok_or_else(|| anyhow!("missing _distance column"))?
            .as_any()
            .downcast_ref::<Float32Array>()
            .ok_or_else(|| anyhow!("_distance is not Float32Array"))?;

        let distance = distances.value(0);
        if distance <= 1.0 - threshold {
            let names = batch
                .column_by_name("topic_name")
                .ok_or_else(|| anyhow!("missing topic_name column"))?
                .as_any()
                .downcast_ref::<StringArray>()
                .ok_or_else(|| anyhow!("topic_name is not StringArray"))?;
            Ok(Some(names.value(0).to_string()))
        } else {
            Ok(None)
        }
    }

    pub async fn list_topics(&self) -> Result<Vec<String>> {
        let batches: Vec<RecordBatch> = self
            .topics
            .query()
            .execute()
            .await?
            .try_collect()
            .await?;

        let mut topics = Vec::new();
        for batch in &batches {
            let names = batch
                .column_by_name("topic_name")
                .ok_or_else(|| anyhow!("missing topic_name column"))?
                .as_any()
                .downcast_ref::<StringArray>()
                .ok_or_else(|| anyhow!("topic_name is not StringArray"))?;
            for i in 0..names.len() {
                topics.push(names.value(i).to_string());
            }
        }
        Ok(topics)
    }

    pub async fn insert_qa(
        &self,
        question: &str,
        answer: &str,
        topic: &str,
        vector: &[f32],
    ) -> Result<()> {
        let schema = qa_schema();
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(StringArray::from(vec![question])),
                Arc::new(StringArray::from(vec![answer])),
                Arc::new(StringArray::from(vec![topic])),
                Arc::new(BooleanArray::from(vec![false])),
                make_vector_array(vector),
            ],
        )?;
        let batches = RecordBatchIterator::new(vec![Ok(batch)], schema);
        self.qa_records.add(Box::new(batches)).execute().await?;
        Ok(())
    }

    pub async fn search_qa(
        &self,
        vector: &[f32],
        topic: &str,
        limit: usize,
    ) -> Result<Vec<QaRecord>> {
        let batches: Vec<RecordBatch> = self
            .qa_records
            .query()
            .nearest_to(vector)?
            .only_if(format!("topic = '{}' AND merged = false", topic))
            .limit(limit)
            .execute()
            .await?
            .try_collect()
            .await?;

        parse_qa_batches(&batches)
    }

    pub async fn find_similar_qa(
        &self,
        vector: &[f32],
        topic: &str,
        threshold: f32,
    ) -> Result<Vec<QaRecord>> {
        let batches: Vec<RecordBatch> = self
            .qa_records
            .query()
            .nearest_to(vector)?
            .only_if(format!("topic = '{}' AND merged = false", topic))
            .limit(50)
            .execute()
            .await?
            .try_collect()
            .await?;

        let all = parse_qa_batches(&batches)?;
        let max_distance = 1.0 - threshold;
        Ok(all
            .into_iter()
            .filter(|r| r.score <= max_distance)
            .collect())
    }

    pub async fn find_nearest_qa_global(&self, vector: &[f32]) -> Result<Option<QaRecord>> {
        let batches: Vec<RecordBatch> = self
            .qa_records
            .query()
            .nearest_to(vector)?
            .limit(1)
            .execute()
            .await?
            .try_collect()
            .await?;

        Ok(parse_qa_batches(&batches)?.into_iter().next())
    }

    pub async fn find_nearest_knowledge_global(
        &self,
        vector: &[f32],
    ) -> Result<Option<KnowledgeRecord>> {
        let batches: Vec<RecordBatch> = self
            .knowledge
            .query()
            .nearest_to(vector)?
            .limit(1)
            .execute()
            .await?
            .try_collect()
            .await?;

        Ok(parse_knowledge_batches(&batches)?.into_iter().next())
    }

    pub async fn mark_merged(&self, questions: &[String]) -> Result<()> {
        for q in questions {
            let escaped = q.replace('\'', "''");
            self.qa_records
                .update()
                .only_if(format!("question = '{}'", escaped))
                .column("merged", "true")
                .execute()
                .await?;
        }
        Ok(())
    }

    pub async fn insert_knowledge(
        &self,
        text: &str,
        topic: &str,
        sources: &[String],
        vector: &[f32],
    ) -> Result<()> {
        let schema = knowledge_schema();

        let mut list_builder = ListBuilder::new(StringBuilder::new());
        for src in sources {
            list_builder.values().append_value(src);
        }
        list_builder.append(true);
        let source_array = Arc::new(list_builder.finish());

        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(StringArray::from(vec![text])),
                Arc::new(StringArray::from(vec![topic])),
                source_array,
                make_vector_array(vector),
            ],
        )?;
        let batches = RecordBatchIterator::new(vec![Ok(batch)], schema);
        self.knowledge.add(Box::new(batches)).execute().await?;
        Ok(())
    }

    pub async fn search_knowledge(
        &self,
        vector: &[f32],
        topic: &str,
        limit: usize,
    ) -> Result<Vec<KnowledgeRecord>> {
        let batches: Vec<RecordBatch> = self
            .knowledge
            .query()
            .nearest_to(vector)?
            .only_if(format!("topic = '{}'", topic))
            .limit(limit)
            .execute()
            .await?
            .try_collect()
            .await?;

        parse_knowledge_batches(&batches)
    }

    pub async fn dump_topics(&self) -> Result<Vec<TopicEntry>> {
        let batches: Vec<RecordBatch> = self
            .topics
            .query()
            .execute()
            .await?
            .try_collect()
            .await?;

        let mut entries = Vec::new();
        for batch in &batches {
            let names = batch
                .column_by_name("topic_name")
                .ok_or_else(|| anyhow!("missing topic_name column"))?
                .as_any()
                .downcast_ref::<StringArray>()
                .ok_or_else(|| anyhow!("topic_name is not StringArray"))?;

            for i in 0..batch.num_rows() {
                entries.push(TopicEntry {
                    topic_name: names.value(i).to_string(),
                });
            }
        }
        Ok(entries)
    }

    pub async fn dump_qa(&self) -> Result<Vec<QaEntry>> {
        let batches: Vec<RecordBatch> = self
            .qa_records
            .query()
            .execute()
            .await?
            .try_collect()
            .await?;

        let mut entries = Vec::new();
        for batch in &batches {
            let questions = batch
                .column_by_name("question")
                .ok_or_else(|| anyhow!("missing question column"))?
                .as_any()
                .downcast_ref::<StringArray>()
                .ok_or_else(|| anyhow!("question is not StringArray"))?;

            let answers = batch
                .column_by_name("answer")
                .ok_or_else(|| anyhow!("missing answer column"))?
                .as_any()
                .downcast_ref::<StringArray>()
                .ok_or_else(|| anyhow!("answer is not StringArray"))?;

            let topics = batch
                .column_by_name("topic")
                .ok_or_else(|| anyhow!("missing topic column"))?
                .as_any()
                .downcast_ref::<StringArray>()
                .ok_or_else(|| anyhow!("topic is not StringArray"))?;

            let merged = batch
                .column_by_name("merged")
                .ok_or_else(|| anyhow!("missing merged column"))?
                .as_any()
                .downcast_ref::<BooleanArray>()
                .ok_or_else(|| anyhow!("merged is not BooleanArray"))?;

            for i in 0..batch.num_rows() {
                entries.push(QaEntry {
                    question: questions.value(i).to_string(),
                    answer: answers.value(i).to_string(),
                    topic: topics.value(i).to_string(),
                    merged: merged.value(i),
                    created_at: None,
                });
            }
        }
        Ok(entries)
    }

    pub async fn dump_knowledge(&self) -> Result<Vec<KnowledgeEntry>> {
        let batches: Vec<RecordBatch> = self
            .knowledge
            .query()
            .execute()
            .await?
            .try_collect()
            .await?;

        let mut entries = Vec::new();
        for batch in &batches {
            let texts = batch
                .column_by_name("knowledge_text")
                .ok_or_else(|| anyhow!("missing knowledge_text column"))?
                .as_any()
                .downcast_ref::<StringArray>()
                .ok_or_else(|| anyhow!("knowledge_text is not StringArray"))?;

            let topics = batch
                .column_by_name("topic")
                .ok_or_else(|| anyhow!("missing topic column"))?
                .as_any()
                .downcast_ref::<StringArray>()
                .ok_or_else(|| anyhow!("topic is not StringArray"))?;

            let source_lists = batch
                .column_by_name("source_questions")
                .ok_or_else(|| anyhow!("missing source_questions column"))?
                .as_any()
                .downcast_ref::<ListArray>()
                .ok_or_else(|| anyhow!("source_questions is not ListArray"))?;

            for i in 0..batch.num_rows() {
                let source_arr = source_lists.value(i);
                let source_strings = source_arr
                    .as_any()
                    .downcast_ref::<StringArray>()
                    .ok_or_else(|| anyhow!("source_questions items are not StringArray"))?;
                let source_questions: Vec<String> = (0..source_strings.len())
                    .map(|j| source_strings.value(j).to_string())
                    .collect();

                entries.push(KnowledgeEntry {
                    knowledge_text: texts.value(i).to_string(),
                    topic: topics.value(i).to_string(),
                    source_questions,
                    created_at: None,
                });
            }
        }
        Ok(entries)
    }

    pub async fn has_topic(&self, name: &str) -> Result<bool> {
        let batches: Vec<RecordBatch> = self
            .topics
            .query()
            .only_if(format!("topic_name = '{}'", name.replace('\'', "''")))
            .limit(1)
            .execute()
            .await?
            .try_collect()
            .await?;
        Ok(!batches.is_empty() && batches[0].num_rows() > 0)
    }

    pub async fn has_qa(&self, question: &str, topic: &str) -> Result<bool> {
        let batches: Vec<RecordBatch> = self
            .qa_records
            .query()
            .only_if(format!(
                "question = '{}' AND topic = '{}'",
                question.replace('\'', "''"),
                topic.replace('\'', "''")
            ))
            .limit(1)
            .execute()
            .await?
            .try_collect()
            .await?;
        Ok(!batches.is_empty() && batches[0].num_rows() > 0)
    }

    pub async fn has_knowledge(&self, text: &str, topic: &str) -> Result<bool> {
        let batches: Vec<RecordBatch> = self
            .knowledge
            .query()
            .only_if(format!(
                "knowledge_text = '{}' AND topic = '{}'",
                text.replace('\'', "''"),
                topic.replace('\'', "''")
            ))
            .limit(1)
            .execute()
            .await?
            .try_collect()
            .await?;
        Ok(!batches.is_empty() && batches[0].num_rows() > 0)
    }

    pub async fn insert_qa_with_merged(
        &self,
        question: &str,
        answer: &str,
        topic: &str,
        merged: bool,
        vector: &[f32],
    ) -> Result<()> {
        let schema = qa_schema();
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(StringArray::from(vec![question])),
                Arc::new(StringArray::from(vec![answer])),
                Arc::new(StringArray::from(vec![topic])),
                Arc::new(BooleanArray::from(vec![merged])),
                make_vector_array(vector),
            ],
        )?;
        let batches = RecordBatchIterator::new(vec![Ok(batch)], schema);
        self.qa_records.add(Box::new(batches)).execute().await?;
        Ok(())
    }

    pub async fn delete_qa(&self, question: &str, topic: &str) -> Result<()> {
        self.qa_records
            .delete(&format!(
                "question = '{}' AND topic = '{}'",
                question.replace('\'', "''"),
                topic.replace('\'', "''")
            ))
            .await?;
        Ok(())
    }

    pub async fn delete_knowledge(&self, text: &str, topic: &str) -> Result<()> {
        self.knowledge
            .delete(&format!(
                "knowledge_text = '{}' AND topic = '{}'",
                text.replace('\'', "''"),
                topic.replace('\'', "''")
            ))
            .await?;
        Ok(())
    }
}

fn parse_qa_batches(batches: &[RecordBatch]) -> Result<Vec<QaRecord>> {
    let mut records = Vec::new();
    for batch in batches {
        let questions = batch
            .column_by_name("question")
            .ok_or_else(|| anyhow!("missing question column"))?
            .as_any()
            .downcast_ref::<StringArray>()
            .ok_or_else(|| anyhow!("question is not StringArray"))?;

        let answers = batch
            .column_by_name("answer")
            .ok_or_else(|| anyhow!("missing answer column"))?
            .as_any()
            .downcast_ref::<StringArray>()
            .ok_or_else(|| anyhow!("answer is not StringArray"))?;

        let topics = batch
            .column_by_name("topic")
            .ok_or_else(|| anyhow!("missing topic column"))?
            .as_any()
            .downcast_ref::<StringArray>()
            .ok_or_else(|| anyhow!("topic is not StringArray"))?;

        let merged = batch
            .column_by_name("merged")
            .ok_or_else(|| anyhow!("missing merged column"))?
            .as_any()
            .downcast_ref::<BooleanArray>()
            .ok_or_else(|| anyhow!("merged is not BooleanArray"))?;

        let distances = batch
            .column_by_name("_distance")
            .and_then(|c| c.as_any().downcast_ref::<Float32Array>().cloned());

        for i in 0..batch.num_rows() {
            records.push(QaRecord {
                question: questions.value(i).to_string(),
                answer: answers.value(i).to_string(),
                topic: topics.value(i).to_string(),
                merged: merged.value(i),
                score: distances.as_ref().map_or(0.0, |d| d.value(i)),
            });
        }
    }
    Ok(records)
}

fn parse_knowledge_batches(batches: &[RecordBatch]) -> Result<Vec<KnowledgeRecord>> {
    let mut records = Vec::new();
    for batch in batches {
        let texts = batch
            .column_by_name("knowledge_text")
            .ok_or_else(|| anyhow!("missing knowledge_text column"))?
            .as_any()
            .downcast_ref::<StringArray>()
            .ok_or_else(|| anyhow!("knowledge_text is not StringArray"))?;

        let topics = batch
            .column_by_name("topic")
            .ok_or_else(|| anyhow!("missing topic column"))?
            .as_any()
            .downcast_ref::<StringArray>()
            .ok_or_else(|| anyhow!("topic is not StringArray"))?;

        let source_lists = batch
            .column_by_name("source_questions")
            .ok_or_else(|| anyhow!("missing source_questions column"))?
            .as_any()
            .downcast_ref::<ListArray>()
            .ok_or_else(|| anyhow!("source_questions is not ListArray"))?;

        let distances = batch
            .column_by_name("_distance")
            .and_then(|c| c.as_any().downcast_ref::<Float32Array>().cloned());

        for i in 0..batch.num_rows() {
            let source_arr = source_lists.value(i);
            let source_strings = source_arr
                .as_any()
                .downcast_ref::<StringArray>()
                .ok_or_else(|| anyhow!("source_questions items are not StringArray"))?;
            let source_questions: Vec<String> = (0..source_strings.len())
                .map(|j| source_strings.value(j).to_string())
                .collect();

            records.push(KnowledgeRecord {
                knowledge_text: texts.value(i).to_string(),
                topic: topics.value(i).to_string(),
                source_questions,
                score: distances.as_ref().map_or(0.0, |d| d.value(i)),
            });
        }
    }
    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn test_storage() -> Storage {
        let dir = tempfile::tempdir().unwrap();
        Storage::open(dir.path().to_str().unwrap()).await.unwrap()
    }

    fn fake_vector(seed: f32) -> Vec<f32> {
        let mut v: Vec<f32> = (0..384)
            .map(|i| (seed + i as f32 * 0.01).sin())
            .collect();
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        v.iter_mut().for_each(|x| *x /= norm);
        v
    }

    #[tokio::test]
    async fn test_open_creates_tables() {
        let storage = test_storage().await;
        let topics = storage.list_topics().await.unwrap();
        assert!(topics.is_empty());
    }

    #[tokio::test]
    async fn test_topic_lifecycle() {
        let storage = test_storage().await;
        let vec = fake_vector(1.0);
        storage.create_topic("Rust编程", &vec).await.unwrap();

        let topics = storage.list_topics().await.unwrap();
        assert_eq!(topics, vec!["Rust编程"]);

        let found = storage.find_similar_topic(&vec, 0.8).await.unwrap();
        assert_eq!(found, Some("Rust编程".to_string()));
    }

    #[tokio::test]
    async fn test_qa_insert_search() {
        let storage = test_storage().await;
        let vec = fake_vector(2.0);
        storage
            .insert_qa("What is Rust?", "A systems language", "programming", &vec)
            .await
            .unwrap();
        let results = storage.search_qa(&vec, "programming", 5).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].question, "What is Rust?");
        assert_eq!(results[0].answer, "A systems language");
    }

    #[tokio::test]
    async fn test_qa_merged_excluded() {
        let storage = test_storage().await;
        let vec = fake_vector(3.0);
        storage
            .insert_qa("Q1", "A1", "topic1", &vec)
            .await
            .unwrap();
        storage
            .mark_merged(&["Q1".to_string()])
            .await
            .unwrap();
        let results = storage.search_qa(&vec, "topic1", 5).await.unwrap();
        assert!(results.is_empty(), "Merged QA should not appear in search");
    }

    #[tokio::test]
    async fn test_knowledge_insert_search() {
        let storage = test_storage().await;
        let vec = fake_vector(4.0);
        storage
            .insert_knowledge(
                "Rust is a systems programming language",
                "programming",
                &[
                    "What is Rust?".to_string(),
                    "Tell me about Rust".to_string(),
                ],
                &vec,
            )
            .await
            .unwrap();
        let results = storage
            .search_knowledge(&vec, "programming", 5)
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].knowledge_text,
            "Rust is a systems programming language"
        );
        assert_eq!(results[0].source_questions.len(), 2);
    }
}
