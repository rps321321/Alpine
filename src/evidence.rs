use crate::identity::sha256_bytes;
use rusqlite::types::Type;
use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};
use serde::Serialize;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::Duration;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS runs (
    id TEXT PRIMARY KEY,
    started_at TEXT NOT NULL,
    finished_at TEXT,
    status TEXT NOT NULL,
    kind TEXT NOT NULL,
    profile TEXT NOT NULL,
    git_commit TEXT,
    hardware_manifest TEXT,
    model_sha256 TEXT,
    backend_commit TEXT,
    config_json TEXT NOT NULL,
    summary_json TEXT,
    notes TEXT
);
CREATE TABLE IF NOT EXISTS samples (
    run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    workload TEXT NOT NULL,
    iteration INTEGER NOT NULL,
    warmup INTEGER NOT NULL DEFAULT 0,
    prompt_tokens INTEGER,
    generated_tokens INTEGER,
    prefill_tps REAL,
    decode_tps REAL,
    ttft_ms REAL,
    latency_ms REAL,
    output_sha256 TEXT,
    quality_pass INTEGER,
    vram_peak_mib REAL,
    gpu_util_mean REAL,
    gpu_power_mean_w REAL,
    gpu_temp_max_c REAL,
    process_working_set_mib REAL,
    raw_json TEXT NOT NULL,
    PRIMARY KEY (run_id, workload, iteration, warmup)
);
CREATE INDEX IF NOT EXISTS samples_profile_lookup ON samples(run_id, workload, warmup);
CREATE TABLE IF NOT EXISTS artifacts (
    run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    path TEXT NOT NULL,
    sha256 TEXT,
    PRIMARY KEY (run_id, kind, path)
);
"#;

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

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MeasuredSample {
    pub workload: String,
    pub iteration: u32,
    pub prefill_tps: Option<f64>,
    pub decode_tps: Option<f64>,
    pub output_sha256: Option<String>,
    pub quality_pass: Option<bool>,
}

#[derive(Debug, Clone)]
pub(crate) struct NewRun {
    pub id: String,
    pub started_at: String,
    pub kind: String,
    pub profile: String,
    pub git_commit: String,
    pub hardware_manifest: String,
    pub model_sha256: String,
    pub backend_commit: String,
    pub config: Value,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TerminalStatus {
    Passed,
    FailedQuality,
    Error,
}

impl TerminalStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::FailedQuality => "failed-quality",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct SampleRecord {
    pub workload: String,
    pub iteration: u32,
    pub warmup: bool,
    pub prompt_tokens: Option<i64>,
    pub generated_tokens: Option<i64>,
    pub prefill_tps: Option<f64>,
    pub decode_tps: Option<f64>,
    pub ttft_ms: Option<f64>,
    pub latency_ms: Option<f64>,
    pub output_sha256: Option<String>,
    pub quality_pass: Option<bool>,
    pub vram_peak_mib: Option<f64>,
    pub gpu_util_mean: Option<f64>,
    pub gpu_power_mean_w: Option<f64>,
    pub gpu_temp_max_c: Option<f64>,
    pub process_working_set_mib: Option<f64>,
    pub raw: Value,
}

pub struct EvidenceStore {
    path: PathBuf,
    connection: Connection,
}

pub(crate) struct EvidenceWriter {
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
        let identity = identity_from_parts(
            git_commit.clone(),
            model_sha256.clone(),
            &config,
            &config_json,
        );
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

    pub(crate) fn measured_samples(&self, id: &str) -> Result<Vec<MeasuredSample>, String> {
        validate_token("run id", id)?;
        let mut statement = self
            .connection
            .prepare(
                "SELECT workload, iteration, prefill_tps, decode_tps, output_sha256, quality_pass
                 FROM samples WHERE run_id=?1 AND warmup=0
                 ORDER BY workload, iteration",
            )
            .map_err(|error| self.database_error(error))?;
        let rows = statement
            .query_map(params![id], |row| {
                let iteration = row.get::<_, u32>(1)?;
                Ok(MeasuredSample {
                    workload: row.get(0)?,
                    iteration,
                    prefill_tps: row.get(2)?,
                    decode_tps: row.get(3)?,
                    output_sha256: row.get(4)?,
                    quality_pass: row.get(5)?,
                })
            })
            .map_err(|error| self.database_error(error))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| self.database_error(error))
    }

    fn database_error(&self, error: rusqlite::Error) -> String {
        format!("evidence database {}: {error}", self.path.display())
    }
}

impl EvidenceWriter {
    pub(crate) fn open(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|error| {
                    format!(
                        "failed to create evidence directory {}: {error}",
                        parent.display()
                    )
                })?;
            }
        }
        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|error| {
            format!(
                "failed to open evidence database {}: {error}",
                path.display()
            )
        })?;
        connection
            .busy_timeout(Duration::from_secs(30))
            .map_err(|error| format!("failed to configure evidence database: {error}"))?;
        connection
            .pragma_update(None, "foreign_keys", "ON")
            .map_err(|error| format!("failed to enable evidence foreign keys: {error}"))?;
        connection
            .pragma_update(None, "journal_mode", "WAL")
            .map_err(|error| format!("failed to enable evidence WAL: {error}"))?;
        connection
            .pragma_update(None, "synchronous", "FULL")
            .map_err(|error| format!("failed to enable durable evidence writes: {error}"))?;
        connection
            .execute_batch(SCHEMA)
            .map_err(|error| format!("failed to initialize evidence database: {error}"))?;
        Ok(Self {
            path: path.to_path_buf(),
            connection,
        })
    }

    pub(crate) fn begin_run(&mut self, run: &NewRun) -> Result<StoredIdentity, String> {
        validate_token("run id", &run.id)?;
        validate_token("run kind", &run.kind)?;
        validate_token("profile", &run.profile)?;
        validate_git_commit(&run.git_commit)?;
        validate_sha256("model", &run.model_sha256)?;
        let config_json = serde_json::to_string(&run.config)
            .map_err(|error| format!("run {} config cannot be serialized: {error}", run.id))?;
        let identity = identity_from_parts(
            Some(run.git_commit.clone()),
            Some(run.model_sha256.clone()),
            &run.config,
            &config_json,
        );
        identity.validate_complete()?;

        let database_path = self.path.clone();
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| database_error_at(&database_path, error))?;
        transaction
            .execute(
                "INSERT INTO runs (
                    id, started_at, status, kind, profile, git_commit, hardware_manifest,
                    model_sha256, backend_commit, config_json, notes
                ) VALUES (?1, ?2, 'running', ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    run.id,
                    run.started_at,
                    run.kind,
                    run.profile,
                    run.git_commit,
                    run.hardware_manifest,
                    run.model_sha256,
                    run.backend_commit,
                    config_json,
                    run.notes,
                ],
            )
            .map_err(|error| database_error_at(&self.path, error))?;
        transaction
            .commit()
            .map_err(|error| self.database_error(error))?;
        Ok(identity)
    }

    pub(crate) fn record_sample(
        &mut self,
        run_id: &str,
        sample: &SampleRecord,
    ) -> Result<(), String> {
        validate_token("run id", run_id)?;
        validate_token("workload", &sample.workload)?;
        if sample.iteration == 0 {
            return Err("sample iteration must be positive".to_owned());
        }
        if let Some(digest) = &sample.output_sha256 {
            validate_sha256("sample output", digest)?;
        }
        let raw_json = serde_json::to_string(&sample.raw)
            .map_err(|error| format!("sample cannot be serialized: {error}"))?;
        let database_path = self.path.clone();
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| database_error_at(&database_path, error))?;
        require_running(&transaction, run_id, &self.path)?;
        transaction
            .execute(
                "INSERT INTO samples VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9,
                    ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18
                )",
                params![
                    run_id,
                    sample.workload,
                    i64::from(sample.iteration),
                    sample.warmup,
                    sample.prompt_tokens,
                    sample.generated_tokens,
                    sample.prefill_tps,
                    sample.decode_tps,
                    sample.ttft_ms,
                    sample.latency_ms,
                    sample.output_sha256,
                    sample.quality_pass,
                    sample.vram_peak_mib,
                    sample.gpu_util_mean,
                    sample.gpu_power_mean_w,
                    sample.gpu_temp_max_c,
                    sample.process_working_set_mib,
                    raw_json,
                ],
            )
            .map_err(|error| database_error_at(&self.path, error))?;
        transaction
            .commit()
            .map_err(|error| self.database_error(error))
    }

    pub(crate) fn finish_run(
        &mut self,
        run_id: &str,
        finished_at: &str,
        status: TerminalStatus,
        summary: &Value,
    ) -> Result<(), String> {
        validate_token("run id", run_id)?;
        let summary_json = serde_json::to_string(summary)
            .map_err(|error| format!("run {run_id} summary cannot be serialized: {error}"))?;
        let database_path = self.path.clone();
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| database_error_at(&database_path, error))?;
        let changed = transaction
            .execute(
                "UPDATE runs SET finished_at=?1, status=?2, summary_json=?3
                 WHERE id=?4 AND status='running' AND finished_at IS NULL",
                params![finished_at, status.as_str(), summary_json, run_id],
            )
            .map_err(|error| database_error_at(&self.path, error))?;
        if changed != 1 {
            return Err(format!(
                "run {run_id} is missing or has already left the running state"
            ));
        }
        transaction
            .commit()
            .map_err(|error| self.database_error(error))
    }

    fn database_error(&self, error: rusqlite::Error) -> String {
        database_error_at(&self.path, error)
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

    fn validate_complete(&self) -> Result<(), String> {
        let missing = self.missing_fields();
        if !missing.is_empty() {
            return Err(format!(
                "new evidence is missing identity fields: {}",
                missing.join(", ")
            ));
        }
        validate_sha256("hardware", self.hardware.as_deref().unwrap_or_default())?;
        validate_git_commit(self.software.as_deref().unwrap_or_default())?;
        validate_sha256("model", self.model.as_deref().unwrap_or_default())?;
        validate_sha256("runtime", self.runtime.as_deref().unwrap_or_default())?;
        validate_sha256("workload", self.workload.as_deref().unwrap_or_default())?;
        validate_sha256(
            "configuration",
            self.configuration.as_deref().unwrap_or_default(),
        )?;
        validate_sha256("policy", self.policy.as_deref().unwrap_or_default())
    }
}

fn identity_from_parts(
    git_commit: Option<String>,
    model_sha256: Option<String>,
    config: &Value,
    config_json: &str,
) -> StoredIdentity {
    StoredIdentity {
        hardware: json_string(config, "/hardware/sha256"),
        software: json_string(config, "/software/alpine_binary_sha256").or(git_commit),
        model: model_sha256,
        runtime: json_string(config, "/launch/runtime_build_sha256")
            .or_else(|| json_string(config, "/launch/server_sha256")),
        workload: json_string(config, "/benchmark/sha256"),
        configuration: json_string(config, "/identity/configuration_sha256")
            .or_else(|| Some(sha256_bytes(config_json.as_bytes()))),
        policy: json_string(config, "/qualification_policy/sha256"),
    }
}

fn require_running(
    transaction: &rusqlite::Transaction<'_>,
    run_id: &str,
    path: &Path,
) -> Result<(), String> {
    let running = transaction
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM runs WHERE id=?1 AND status='running' AND finished_at IS NULL)",
            params![run_id],
            |row| row.get::<_, bool>(0),
        )
        .map_err(|error| database_error_at(path, error))?;
    if running {
        Ok(())
    } else {
        Err(format!(
            "run {run_id} is missing or has already left the running state"
        ))
    }
}

fn database_error_at(path: &Path, error: rusqlite::Error) -> String {
    format!("evidence database {}: {error}", path.display())
}

fn validate_token(name: &str, value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(format!(
            "{name} must be 1-128 ASCII letters, digits, '.', '_' or '-'"
        ));
    }
    Ok(())
}

fn validate_git_commit(value: &str) -> Result<(), String> {
    if matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err("software identity must be a 40- or 64-character hexadecimal Git commit".to_owned())
    }
}

fn validate_sha256(name: &str, value: &str) -> Result<(), String> {
    if value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(format!(
            "{name} identity must be a 64-character hexadecimal SHA-256"
        ))
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
    use std::sync::{Arc, Barrier};
    use std::thread;

    fn digest(byte: char) -> String {
        std::iter::repeat_n(byte, 64).collect()
    }

    fn writable_run(id: &str) -> NewRun {
        NewRun {
            id: id.to_owned(),
            started_at: "2026-08-20T00:00:00Z".to_owned(),
            kind: "micro".to_owned(),
            profile: "stable-16k".to_owned(),
            git_commit: std::iter::repeat_n('a', 40).collect(),
            hardware_manifest: "inventory/fixture.json".to_owned(),
            model_sha256: digest('b'),
            backend_commit: std::iter::repeat_n('c', 40).collect(),
            config: serde_json::json!({
                "hardware": {"sha256": digest('d')},
                "launch": {"runtime_build_sha256": digest('e')},
                "benchmark": {"sha256": digest('f')},
                "qualification_policy": {"sha256": digest('1')}
            }),
            notes: None,
        }
    }

    fn sample() -> SampleRecord {
        SampleRecord {
            workload: "novel-256".to_owned(),
            iteration: 1,
            warmup: false,
            prompt_tokens: Some(4),
            generated_tokens: Some(1),
            prefill_tps: Some(100.0),
            decode_tps: Some(10.0),
            ttft_ms: Some(5.0),
            latency_ms: Some(100.0),
            output_sha256: Some(digest('2')),
            quality_pass: Some(true),
            vram_peak_mib: Some(1024.0),
            gpu_util_mean: Some(50.0),
            gpu_power_mean_w: Some(100.0),
            gpu_temp_max_c: Some(60.0),
            process_working_set_mib: Some(2048.0),
            raw: serde_json::json!({"content": "A"}),
        }
    }

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

    #[test]
    fn writer_requires_complete_identity_and_one_way_lifecycle() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("results.sqlite3");
        let mut writer = EvidenceWriter::open(&path).unwrap();

        let mut incomplete = writable_run("incomplete");
        incomplete.config["qualification_policy"] = Value::Null;
        assert!(
            writer
                .begin_run(&incomplete)
                .unwrap_err()
                .contains("policy")
        );

        let identity = writer.begin_run(&writable_run("complete")).unwrap();
        assert!(identity.missing_fields().is_empty());
        writer.record_sample("complete", &sample()).unwrap();
        writer
            .finish_run(
                "complete",
                "2026-08-20T00:01:00Z",
                TerminalStatus::Passed,
                &serde_json::json!({"all_quality_pass": true}),
            )
            .unwrap();
        assert!(writer.record_sample("complete", &sample()).is_err());
        assert!(
            writer
                .finish_run(
                    "complete",
                    "2026-08-20T00:02:00Z",
                    TerminalStatus::Error,
                    &serde_json::json!({"error": "late overwrite"}),
                )
                .is_err()
        );

        drop(writer);
        let reader = EvidenceStore::open_read_only(&path).unwrap();
        let evidence = reader.run("complete").unwrap().unwrap();
        assert_eq!(evidence.summary.status, "passed");
        assert_eq!(evidence.summary.sample_count, 1);
        assert!(evidence.identity_complete);
    }

    #[test]
    fn writer_serializes_concurrent_independent_runs() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("results.sqlite3");
        EvidenceWriter::open(&path).unwrap();
        let barrier = Arc::new(Barrier::new(8));
        let workers = (0..8)
            .map(|index| {
                let path = path.clone();
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    let run_id = format!("run-{index}");
                    let mut writer = EvidenceWriter::open(&path).unwrap();
                    barrier.wait();
                    writer.begin_run(&writable_run(&run_id)).unwrap();
                    writer.record_sample(&run_id, &sample()).unwrap();
                    writer
                        .finish_run(
                            &run_id,
                            "2026-08-20T00:01:00Z",
                            TerminalStatus::Passed,
                            &serde_json::json!({"all_quality_pass": true}),
                        )
                        .unwrap();
                })
            })
            .collect::<Vec<_>>();
        for worker in workers {
            worker.join().unwrap();
        }
        let reader = EvidenceStore::open_read_only(&path).unwrap();
        let runs = reader.list_runs(20).unwrap();
        assert_eq!(runs.len(), 8);
        assert!(
            runs.iter()
                .all(|run| run.status == "passed" && run.sample_count == 1)
        );
    }
}
