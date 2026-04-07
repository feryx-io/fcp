//! PostgreSQL implementation of the prompt store.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use fcp_core::prompt::{Prompt, PromptCategory, PromptSourceType, PromptStore};
use fcp_core::{PromptId, TenantId};
use sqlx::{Pool, Postgres, Row};
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn system_time_to_datetime(st: SystemTime) -> DateTime<Utc> {
    let duration = st.duration_since(UNIX_EPOCH).unwrap_or_default();
    DateTime::from_timestamp(duration.as_secs() as i64, duration.subsec_nanos()).unwrap_or_default()
}

fn datetime_to_system_time(dt: DateTime<Utc>) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(dt.timestamp() as u64)
}

/// PostgreSQL-backed prompt store.
#[derive(Clone)]
pub struct PostgresPromptStore {
    pool: Pool<Postgres>,
}

impl PostgresPromptStore {
    pub fn new(pool: Pool<Postgres>) -> Self {
        Self { pool }
    }

    fn row_to_prompt(&self, row: sqlx::postgres::PgRow) -> anyhow::Result<Prompt> {
        let id_str: String = row.try_get("id")?;
        let tenant_str: String = row.try_get("tenant_id")?;
        let category_str: String = row.try_get("category")?;
        let source_type_str: String = row.try_get("source_type")?;
        let audio_path_str: Option<String> = row.try_get("audio_path")?;
        let created_at: DateTime<Utc> = row.try_get("created_at")?;
        let updated_at: DateTime<Utc> = row.try_get("updated_at")?;

        Ok(Prompt {
            id: id_str
                .parse()
                .map_err(|e| anyhow::anyhow!("Invalid prompt id: {}", e))?,
            tenant_id: tenant_str
                .parse()
                .map_err(|e| anyhow::anyhow!("Invalid tenant id: {}", e))?,
            name: row.try_get("name")?,
            description: row.try_get("description")?,
            category: category_str.parse::<PromptCategory>()?,
            language: row.try_get("language")?,
            source_type: source_type_str.parse::<PromptSourceType>()?,
            tts_text: row.try_get("tts_text")?,
            tts_voice: row.try_get("tts_voice")?,
            audio_path: audio_path_str.map(PathBuf::from),
            audio_format: row.try_get("audio_format")?,
            duration_ms: row
                .try_get::<Option<i32>, _>("duration_ms")?
                .map(|v| v as u32),
            file_size: row
                .try_get::<Option<i64>, _>("file_size")?
                .map(|v| v as u64),
            is_system: row.try_get("is_system")?,
            created_at: datetime_to_system_time(created_at),
            updated_at: datetime_to_system_time(updated_at),
        })
    }
}

const PROMPT_COLUMNS: &str = "id, tenant_id, name, description, category, language, source_type,
     tts_text, tts_voice, audio_path, audio_format, duration_ms, file_size,
     is_system, created_at, updated_at";

#[async_trait]
impl PromptStore for PostgresPromptStore {
    async fn get_prompt(&self, id: &PromptId) -> anyhow::Result<Option<Prompt>> {
        let row = sqlx::query(&format!(
            "SELECT {} FROM prompts WHERE id = $1",
            PROMPT_COLUMNS
        ))
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(row) => Ok(Some(self.row_to_prompt(row)?)),
            None => Ok(None),
        }
    }

    async fn get_prompt_by_name(
        &self,
        tenant_id: &TenantId,
        name: &str,
    ) -> anyhow::Result<Option<Prompt>> {
        let row = sqlx::query(&format!(
            "SELECT {} FROM prompts WHERE tenant_id = $1 AND name = $2 ORDER BY language LIMIT 1",
            PROMPT_COLUMNS
        ))
        .bind(tenant_id.to_string())
        .bind(name)
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(row) => Ok(Some(self.row_to_prompt(row)?)),
            None => Ok(None),
        }
    }

    async fn get_prompt_by_name_and_language(
        &self,
        tenant_id: &TenantId,
        name: &str,
        language: &str,
    ) -> anyhow::Result<Option<Prompt>> {
        let row = sqlx::query(&format!(
            "SELECT {} FROM prompts WHERE tenant_id = $1 AND name = $2 AND language = $3",
            PROMPT_COLUMNS
        ))
        .bind(tenant_id.to_string())
        .bind(name)
        .bind(language)
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(row) => Ok(Some(self.row_to_prompt(row)?)),
            None => Ok(None),
        }
    }

    async fn list_prompts(
        &self,
        tenant_id: &TenantId,
        category: Option<PromptCategory>,
        language: Option<&str>,
    ) -> anyhow::Result<Vec<Prompt>> {
        let mut query = format!(
            "SELECT {} FROM prompts WHERE tenant_id = $1",
            PROMPT_COLUMNS
        );
        let mut param_idx = 2;

        if category.is_some() {
            query.push_str(&format!(" AND category = ${}", param_idx));
            param_idx += 1;
        }
        if language.is_some() {
            query.push_str(&format!(" AND language = ${}", param_idx));
        }
        query.push_str(" ORDER BY name, language");

        let mut q = sqlx::query(&query).bind(tenant_id.to_string());
        if let Some(cat) = category {
            q = q.bind(cat.to_string());
        }
        if let Some(lang) = language {
            q = q.bind(lang);
        }

        let rows = q.fetch_all(&self.pool).await?;
        let mut prompts = Vec::new();
        for row in rows {
            prompts.push(self.row_to_prompt(row)?);
        }
        Ok(prompts)
    }

    async fn put_prompt(&self, prompt: Prompt) -> anyhow::Result<()> {
        let now = Utc::now();
        let created_at = system_time_to_datetime(prompt.created_at);
        let audio_path_str = prompt
            .audio_path
            .as_ref()
            .map(|p| p.to_string_lossy().to_string());

        sqlx::query(
            "INSERT INTO prompts (id, tenant_id, name, description, category, language, source_type,
                                  tts_text, tts_voice, audio_path, audio_format, duration_ms,
                                  file_size, is_system, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)
             ON CONFLICT (tenant_id, name, language) DO UPDATE SET
                description = EXCLUDED.description,
                category = EXCLUDED.category,
                source_type = EXCLUDED.source_type,
                tts_text = EXCLUDED.tts_text,
                tts_voice = EXCLUDED.tts_voice,
                audio_path = EXCLUDED.audio_path,
                audio_format = EXCLUDED.audio_format,
                duration_ms = EXCLUDED.duration_ms,
                file_size = EXCLUDED.file_size,
                is_system = EXCLUDED.is_system,
                updated_at = EXCLUDED.updated_at",
        )
        .bind(prompt.id.to_string())
        .bind(prompt.tenant_id.to_string())
        .bind(&prompt.name)
        .bind(&prompt.description)
        .bind(prompt.category.to_string())
        .bind(&prompt.language)
        .bind(prompt.source_type.to_string())
        .bind(&prompt.tts_text)
        .bind(&prompt.tts_voice)
        .bind(&audio_path_str)
        .bind(&prompt.audio_format)
        .bind(prompt.duration_ms.map(|v| v as i32))
        .bind(prompt.file_size.map(|v| v as i64))
        .bind(prompt.is_system)
        .bind(created_at)
        .bind(now)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn delete_prompt(&self, id: &PromptId) -> anyhow::Result<()> {
        sqlx::query("DELETE FROM prompts WHERE id = $1")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}
