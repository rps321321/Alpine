use crate::identity::sha256_bytes;
use rusqlite::types::Type;
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde::Serialize;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RunSummary {
    pub id: String,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub status: String,
    pub kind: String,
    pub profile: String,
    pub sample_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StoredIdentity {
    pub hardware: Option<String>,
    pub software: Option<String>,
    pub model: Option<String>,
    pub runtime: Option<String>,
    pub workload: Option<String>,
    pub configuration: Option<String>,
    pub policy: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RunEvidence {
    pub summary: RunSummary,
    pub git_commit: Option<String>,
    pub hardware_manifest: Option<String>,
    pub model_sha256: Option<String>,
    pub backend_commit: Option<String>,
    pub config: Value,
    pub result_summary: Option<Value>,
    pub notes: Option<String>,
    pub identity: StoredIdentity,
    pub identity_complete: bool,
    pub missing_identity_fields: Vec<String>,
}

pub struct EvidenceStore {
    path: PathBuf,
    connection: Connection,
}

impl EvidenceStore {
    pub fn open_read_only(path: &Path) -> Result<Self, String> {
        if !path.is_file() {
            return Err(format!("evidence database is missing: {}", path.display()));
        }
        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|error| {
            format!(
                "failed to open evidence database {}: {error}",
                path.display()
            )
        })?;
        connection
            .busy_timeout(Duration::from_secs(5))
            .map_err(|error| format!("failed to configure evidence database: {error}"))?;
        Ok(Self {
            path: path.to_path_buf(),
            connection,
        })
    }

    pub fn list_runs(&self, limit: u32) -> Result<Vec<RunSummary>, String> {
        if !(1..=1000).contains(&limit) {
            return Err("run limit must be between 1 and 1000".to_owned());
        }
        let mut statement = self
            .connection
            .prepare(
                "SELECT r.id, r.started_at, r.finished_at, r.status, r.kind, r.profile, \
                 (SELECT COUNT(*) FROM samples s WHERE s.run_id = r.id) \
                 FROM runs r ORDER BY r.started_at DESC LIMIT ?1",
            )
            .map_err(|error| self.database_error(error))?;
        let rows = statement
            .query_map(params![limit], row_to_summary)
            .map_err(|error| self.database_error(error))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| self.database_error(error))
    }

    pub fn run(&self, id: &str) -> Result<Option<RunEvidence>, String> {
        let row = self
            .connection
            .query_row(
                "SELECT r.id, r.started_at, r.finished_at, r.status, r.kind, r.profile, \
                 (SELECT COUNT(*) FROM samples s WHERE s.run_id = r.id), \
                 r.git_commit, r.hardware_manifest, r.model_sha256, r.backend_commit, \
                 r.config_json, r.summary_json, r.notes \
                 FROM runs r WHERE r.id = ?1",
                params![id],
                |row| {
                    let summary = row_to_summary(row)?;
                    Ok((
                        summary,
                        row.get::<_, Option<String>>(7)?,
                        row.get::<_, Option<String>>(8)?,
                        row.get::<_, Option<String>>(9)?,
                        row.get::<_, Option<String>>(10)?,
                        row.get::<_, String>(11)?,
                        row.get::<_, Option<String>>(12)?,
                        row.get::<_, Option<String>>(13)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| self.database_error(error))?;
        let Some((
            summary,
            git_commit,
            hardware_manifest,
            model_sha256,
            backend_commit,
            config_json,
            summary_json,
            notes,
        )) = row
        else {
            return Ok(None);
        };
        let config: Value = serde_json::from_str(&config_json)
            .map_err(|error| format!("run {id} has invalid config JSON: {error}"))?;
        let result_summary = summary_json
            .map(|value| serde_json::from_str(&value))
            .transpose()
            .map_err(|error| format!("run {id} has invalid summary JSON: {error}"))?;
        let identity = StoredIdentity {
            hardware: json_string(&config, "/hardware/sha256"),
            software: git_commit.clone(),
            model: model_sha256.clone(),
            runtime: json_string(&config, "/launch/runtime_build_sha256")
                .or_else(|| json_string(&config, "/launch/server_sha256")),
            workload: json_string(&config, "/benchmark/sha256"),
            configuration: Some(sha256_bytes(config_json.as_bytes())),
            policy: json_string(&config, "/qualification_policy/sha256"),
        };
        let missing_identity_fields = identity.missing_fields();
        Ok(Some(RunEvidence {
            summary,
            git_commit,
            hardware_manifest,
            model_sha256,
            backend_commit,
            config,
            result_summary,
            notes,
            identity_complete: missing_identity_fields.is_empty(),
            missing_identity_fields,
            identity,
        }))
    }

    fn database_error(&self, error: rusqlite::Error) -> String {
        format!("evidence database {}: {error}", self.path.display())
    }
}

impl StoredIdentity {
    fn missing_fields(&self) -> Vec<String> {
        [
            ("hardware", &self.hardware),
            ("software", &self.software),
            ("model", &self.model),
            ("runtime", &self.runtime),
            ("workload", &self.workload),
            ("configuration", &self.configuration),
            ("policy", &self.policy),
        ]
        .into_iter()
        .filter(|(_, value)| value.as_ref().is_none_or(|value| value.trim().is_empty()))
        .map(|(name, _)| name.to_owned())
        .collect()
    }
}

fn row_to_summary(row: &rusqlite::Row<'_>) -> rusqlite::Result<RunSummary> {
    let stored_sample_count: i64 = row.get(6)?;
    let sample_count = u64::try_from(stored_sample_count).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(6, Type::Integer, Box::new(error))
    })?;
    Ok(RunSummary {
        id: row.get(0)?,
        started_at: row.get(1)?,
        finished_at: row.get(2)?,
        status: row.get(3)?,
        kind: row.get(4)?,
        profile: row.get(5)?,
        sample_count,
    })
}

fn json_string(value: &Value, pointer: &str) -> Option<String> {
    value.pointer(pointer)?.as_str().map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn database() -> tempfile::TempDir {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("results.sqlite3");
        let connection = Connection::open(path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE runs (
                    id TEXT PRIMARY KEY, started_at TEXT NOT NULL, finished_at TEXT,
                    status TEXT NOT NULL, kind TEXT NOT NULL, profile TEXT NOT NULL,
                    git_commit TEXT, hardware_manifest TEXT, model_sha256 TEXT,
                    backend_commit TEXT, config_json TEXT NOT NULL, summary_json TEXT, notes TEXT
                );
                CREATE TABLE samples (run_id TEXT NOT NULL);",
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO runs VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                params![
                    "run-1",
                    "2026-08-20T00:00:00Z",
                    "2026-08-20T00:01:00Z",
                    "passed",
                    "micro",
                    "stable-16k",
                    "commit",
                    "inventory.json",
                    "model",
                    "backend",
                    serde_json::json!({
                        "hardware": {"sha256": "hardware"},
                        "launch": {"server_sha256": "runtime"},
                        "benchmark": {"sha256": "workload"}
                    })
                    .to_string(),
                    serde_json::json!({"all_quality_pass": true}).to_string(),
                    "fixture"
                ],
            )
            .unwrap();
        connection
            .execute("INSERT INTO samples VALUES (?1)", params!["run-1"])
            .unwrap();
        directory
    }

    #[test]
    fn reads_existing_runs_without_mutating_the_database() {
        let directory = database();
        let store =
            EvidenceStore::open_read_only(&directory.path().join("results.sqlite3")).unwrap();
        let runs = store.list_runs(10).unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].sample_count, 1);
        let evidence = store.run("run-1").unwrap().unwrap();
        assert_eq!(evidence.identity.runtime.as_deref(), Some("runtime"));
        assert!(!evidence.identity_complete);
        assert_eq!(evidence.missing_identity_fields, vec!["policy"]);
    }

    #[test]
    fn missing_database_and_invalid_limit_are_actionable() {
        let directory = tempfile::tempdir().unwrap();
        assert!(EvidenceStore::open_read_only(&directory.path().join("missing.sqlite3")).is_err());
        let directory = database();
        let store =
            EvidenceStore::open_read_only(&directory.path().join("results.sqlite3")).unwrap();
        assert!(
            store
                .list_runs(0)
                .unwrap_err()
                .contains("between 1 and 1000")
        );
    }
}
