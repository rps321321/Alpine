use crate::clock::UtcTimestamp;
use crate::decision::Decision;
use crate::evidence::EvidenceStore;
use crate::identity::{sha256_bytes, sha256_file};
use crate::locking::InterprocessLock;
use crate::qualification::{
    EvidenceIdentity, QualificationTarget, RunQualificationOptions, RunQualificationReport,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

const EVENT_SCHEMA: u32 = 1;
const MAX_EVENT_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeploymentRoles {
    pub daily_default: String,
    pub rollback_profile: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RollbackDisposition {
    Suspended,
    Revoked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QualificationReference {
    pub final_run_id: String,
    pub tuning_run_ids: Vec<String>,
    pub identity: EvidenceIdentity,
    pub profile_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum DeploymentEventPayload {
    Bootstrap {
        roles: DeploymentRoles,
    },
    Promotion {
        profile: String,
        before: DeploymentRoles,
        after: DeploymentRoles,
        qualification: Box<QualificationReference>,
    },
    Rollback {
        profile: String,
        promotion_event_id: String,
        before: DeploymentRoles,
        after: DeploymentRoles,
        disposition: RollbackDisposition,
    },
    Incident {
        profile: String,
        promotion_event_id: Option<String>,
    },
    IncidentResolution {
        profile: String,
        suspension_event_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeploymentEvent {
    pub schema: u32,
    pub sequence: u64,
    pub id: String,
    pub created_at: String,
    pub operator: String,
    pub reason: String,
    pub previous_event_sha256: Option<String>,
    pub payload: DeploymentEventPayload,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OpenSuspension {
    pub event_id: String,
    pub profile: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeploymentStatus {
    pub schema: u32,
    pub initialized: bool,
    pub roles: Option<DeploymentRoles>,
    pub last_sequence: Option<u64>,
    pub last_event_id: Option<String>,
    pub event_count: usize,
    pub open_suspensions: Vec<OpenSuspension>,
}

#[derive(Debug, Clone)]
pub struct BootstrapDeploymentOptions {
    pub install_root: PathBuf,
    pub daily_default: String,
    pub rollback_profile: String,
    pub operator: String,
    pub reason: String,
    pub lock_timeout: Duration,
}

#[derive(Debug, Clone)]
pub struct PromoteOptions {
    pub repository_root: PathBuf,
    pub install_root: PathBuf,
    pub database: PathBuf,
    pub final_run_id: String,
    pub tuning_run_ids: Vec<String>,
    pub profile: String,
    pub expected_daily_default: String,
    pub operator: String,
    pub reason: String,
    pub support_timeout: Duration,
    pub lock_timeout: Duration,
}

#[derive(Debug, Clone)]
pub struct RollbackDeploymentOptions {
    pub install_root: PathBuf,
    pub expected_daily_default: String,
    pub promotion_event_id: String,
    pub disposition: RollbackDisposition,
    pub operator: String,
    pub reason: String,
    pub lock_timeout: Duration,
}

#[derive(Debug, Clone)]
pub struct RecordIncidentOptions {
    pub install_root: PathBuf,
    pub profile: String,
    pub promotion_event_id: Option<String>,
    pub operator: String,
    pub reason: String,
    pub lock_timeout: Duration,
}

#[derive(Debug, Clone)]
pub struct ResolveIncidentOptions {
    pub install_root: PathBuf,
    pub suspension_event_id: String,
    pub profile: String,
    pub operator: String,
    pub resolution: String,
    pub lock_timeout: Duration,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeploymentChangeReport {
    pub event: DeploymentEvent,
    pub event_path: PathBuf,
    pub event_sha256: String,
    pub status: DeploymentStatus,
    pub qualification: Option<RunQualificationReport>,
}

#[derive(Debug)]
struct Replay {
    status: DeploymentStatus,
    last_digest: Option<String>,
    events: BTreeMap<String, DeploymentEvent>,
}

pub fn status(install_root: &Path) -> Result<DeploymentStatus, String> {
    Ok(replay(install_root)?.status)
}

pub fn daily_default(install_root: &Path) -> Result<Option<String>, String> {
    Ok(replay(install_root)?
        .status
        .roles
        .map(|roles| roles.daily_default))
}

pub fn bootstrap(options: &BootstrapDeploymentOptions) -> Result<DeploymentChangeReport, String> {
    validate_profile_name(&options.daily_default)?;
    validate_profile_name(&options.rollback_profile)?;
    validate_actor_and_reason(&options.operator, &options.reason)?;
    let _lock = deployment_lock(&options.install_root, options.lock_timeout)?;
    let current = replay(&options.install_root)?;
    if current.status.initialized {
        return Err("deployment history is already initialized".to_owned());
    }
    ensure_profile_exists(&options.install_root, &options.daily_default)?;
    ensure_profile_exists(&options.install_root, &options.rollback_profile)?;
    append(
        &options.install_root,
        &current,
        &options.operator,
        &options.reason,
        DeploymentEventPayload::Bootstrap {
            roles: DeploymentRoles {
                daily_default: options.daily_default.clone(),
                rollback_profile: options.rollback_profile.clone(),
            },
        },
        None,
    )
}

pub fn promote(options: &PromoteOptions) -> Result<DeploymentChangeReport, String> {
    validate_profile_name(&options.profile)?;
    validate_profile_name(&options.expected_daily_default)?;
    validate_actor_and_reason(&options.operator, &options.reason)?;
    let _lock = deployment_lock(&options.install_root, options.lock_timeout)?;
    let current = replay(&options.install_root)?;
    let roles = required_roles(&current)?;
    if roles.daily_default != options.expected_daily_default {
        return Err(format!(
            "deployment daily_default changed: expected {}, observed {}",
            options.expected_daily_default, roles.daily_default
        ));
    }
    if roles.daily_default == options.profile {
        return Err(format!("{} is already the daily_default", options.profile));
    }
    if current
        .status
        .open_suspensions
        .iter()
        .any(|suspension| suspension.profile == options.profile)
    {
        return Err(format!(
            "{} has unresolved deployment suspensions and cannot be promoted",
            options.profile
        ));
    }
    ensure_profile_exists(&options.install_root, &options.profile)?;

    let qualification = crate::qualification::qualify_run(&RunQualificationOptions {
        repository_root: options.repository_root.clone(),
        install_root: options.install_root.clone(),
        database: options.database.clone(),
        final_run_id: options.final_run_id.clone(),
        tuning_run_ids: options.tuning_run_ids.clone(),
        target: QualificationTarget::Production,
        support_timeout: options.support_timeout,
    })?;
    if qualification.decision != Decision::Qualified {
        return Err(format!(
            "production qualification is not current: {:?}",
            qualification.decision
        ));
    }
    let identity = qualification
        .identity
        .clone()
        .ok_or_else(|| "qualified deployment has no complete Evidence Identity".to_owned())?;
    let store = EvidenceStore::open_read_only(&options.database)?;
    let anchor = store
        .run(&options.final_run_id)?
        .ok_or_else(|| format!("evidence run not found: {}", options.final_run_id))?;
    if anchor.summary.profile != options.profile {
        return Err(format!(
            "final evidence Profile {} does not match requested promotion {}",
            anchor.summary.profile, options.profile
        ));
    }
    let profile_sha256 = anchor
        .config
        .pointer("/launch/profile_sha256")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "final evidence has no Profile file identity".to_owned())?
        .to_owned();
    let current_profile_sha256 = sha256_file(
        &options
            .install_root
            .join("profiles")
            .join(format!("{}.json", options.profile)),
    )?;
    if profile_sha256 != current_profile_sha256 {
        return Err("qualified Profile bytes changed before Promotion".to_owned());
    }
    let after = DeploymentRoles {
        daily_default: options.profile.clone(),
        rollback_profile: roles.rollback_profile.clone(),
    };
    append(
        &options.install_root,
        &current,
        &options.operator,
        &options.reason,
        DeploymentEventPayload::Promotion {
            profile: options.profile.clone(),
            before: roles,
            after,
            qualification: Box::new(QualificationReference {
                final_run_id: options.final_run_id.clone(),
                tuning_run_ids: options.tuning_run_ids.clone(),
                identity,
                profile_sha256,
            }),
        },
        Some(qualification),
    )
}

pub fn rollback(options: &RollbackDeploymentOptions) -> Result<DeploymentChangeReport, String> {
    validate_profile_name(&options.expected_daily_default)?;
    validate_event_id(&options.promotion_event_id)?;
    validate_actor_and_reason(&options.operator, &options.reason)?;
    let _lock = deployment_lock(&options.install_root, options.lock_timeout)?;
    let current = replay(&options.install_root)?;
    let roles = required_roles(&current)?;
    if roles.daily_default != options.expected_daily_default {
        return Err(format!(
            "deployment daily_default changed: expected {}, observed {}",
            options.expected_daily_default, roles.daily_default
        ));
    }
    let promotion = current
        .events
        .get(&options.promotion_event_id)
        .ok_or_else(|| "referenced Promotion event does not exist".to_owned())?;
    let promoted_profile = match &promotion.payload {
        DeploymentEventPayload::Promotion { profile, .. } => profile.clone(),
        _ => return Err("rollback reference is not a Promotion event".to_owned()),
    };
    if promoted_profile != roles.daily_default {
        return Err("referenced Promotion is not the current daily_default".to_owned());
    }
    let rollback_profile = roles.rollback_profile.clone();
    ensure_profile_exists(&options.install_root, &rollback_profile)?;
    let after = DeploymentRoles {
        daily_default: rollback_profile,
        rollback_profile: roles.rollback_profile.clone(),
    };
    append(
        &options.install_root,
        &current,
        &options.operator,
        &options.reason,
        DeploymentEventPayload::Rollback {
            profile: promoted_profile,
            promotion_event_id: options.promotion_event_id.clone(),
            before: roles,
            after,
            disposition: options.disposition,
        },
        None,
    )
}

pub fn record_incident(options: &RecordIncidentOptions) -> Result<DeploymentChangeReport, String> {
    validate_profile_name(&options.profile)?;
    validate_actor_and_reason(&options.operator, &options.reason)?;
    if let Some(id) = options.promotion_event_id.as_deref() {
        validate_event_id(id)?;
    }
    let _lock = deployment_lock(&options.install_root, options.lock_timeout)?;
    let current = replay(&options.install_root)?;
    required_roles(&current)?;
    ensure_profile_exists(&options.install_root, &options.profile)?;
    if let Some(id) = options.promotion_event_id.as_deref() {
        let event = current
            .events
            .get(id)
            .ok_or_else(|| "referenced Promotion event does not exist".to_owned())?;
        match &event.payload {
            DeploymentEventPayload::Promotion { profile, .. } if profile == &options.profile => {}
            _ => {
                return Err(
                    "Incident Promotion reference does not match the affected Profile".to_owned(),
                );
            }
        }
    }
    append(
        &options.install_root,
        &current,
        &options.operator,
        &options.reason,
        DeploymentEventPayload::Incident {
            profile: options.profile.clone(),
            promotion_event_id: options.promotion_event_id.clone(),
        },
        None,
    )
}

pub fn resolve_incident(
    options: &ResolveIncidentOptions,
) -> Result<DeploymentChangeReport, String> {
    validate_event_id(&options.suspension_event_id)?;
    validate_profile_name(&options.profile)?;
    validate_actor_and_reason(&options.operator, &options.resolution)?;
    let _lock = deployment_lock(&options.install_root, options.lock_timeout)?;
    let current = replay(&options.install_root)?;
    required_roles(&current)?;
    let suspension = current
        .status
        .open_suspensions
        .iter()
        .find(|suspension| suspension.event_id == options.suspension_event_id)
        .ok_or_else(|| "referenced suspension is not open".to_owned())?;
    if suspension.profile != options.profile {
        return Err("incident resolution Profile does not match the suspension".to_owned());
    }
    append(
        &options.install_root,
        &current,
        &options.operator,
        &options.resolution,
        DeploymentEventPayload::IncidentResolution {
            profile: options.profile.clone(),
            suspension_event_id: options.suspension_event_id.clone(),
        },
        None,
    )
}

fn append(
    install_root: &Path,
    current: &Replay,
    operator: &str,
    reason: &str,
    payload: DeploymentEventPayload,
    qualification: Option<RunQualificationReport>,
) -> Result<DeploymentChangeReport, String> {
    let sequence = current.status.last_sequence.map_or(Ok(0), |value| {
        value
            .checked_add(1)
            .ok_or_else(|| "deployment sequence overflow".to_owned())
    })?;
    let now = UtcTimestamp::now()?;
    let event = DeploymentEvent {
        schema: EVENT_SCHEMA,
        sequence,
        id: uuid::Uuid::new_v4().simple().to_string(),
        created_at: now.rfc3339(),
        operator: operator.to_owned(),
        reason: reason.to_owned(),
        previous_event_sha256: current.last_digest.clone(),
        payload,
    };
    let bytes = serde_json::to_vec_pretty(&event)
        .map_err(|error| format!("failed to encode deployment event: {error}"))?;
    let mut rendered = bytes;
    rendered.push(b'\n');
    let directory = event_directory(install_root);
    std::fs::create_dir_all(&directory).map_err(|error| {
        format!(
            "failed to create deployment event directory {}: {error}",
            directory.display()
        )
    })?;
    let path = directory.join(format!("{sequence:020}-{}.json", event.id));
    if path.exists() {
        return Err(format!(
            "deployment event path already exists: {}",
            path.display()
        ));
    }
    let mut temporary = tempfile::NamedTempFile::new_in(&directory)
        .map_err(|error| format!("failed to stage deployment event: {error}"))?;
    temporary
        .write_all(&rendered)
        .and_then(|()| temporary.as_file_mut().sync_all())
        .map_err(|error| format!("failed to durably stage deployment event: {error}"))?;
    temporary
        .persist(&path)
        .map_err(|error| format!("failed to publish deployment event: {}", error.error))?;
    let digest = sha256_bytes(&rendered);
    let status = replay(install_root)?.status;
    if status.last_event_id.as_deref() != Some(event.id.as_str()) {
        return Err(
            "published deployment event did not become the derived current state".to_owned(),
        );
    }
    Ok(DeploymentChangeReport {
        event,
        event_path: path,
        event_sha256: digest,
        status,
        qualification,
    })
}

fn replay(install_root: &Path) -> Result<Replay, String> {
    let directory = event_directory(install_root);
    if !directory.exists() {
        return Ok(empty_replay());
    }
    let metadata = std::fs::symlink_metadata(&directory).map_err(|error| {
        format!(
            "failed to inspect deployment event directory {}: {error}",
            directory.display()
        )
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("deployment event path must be a real directory".to_owned());
    }
    let mut paths = std::fs::read_dir(&directory)
        .map_err(|error| format!("failed to enumerate deployment events: {error}"))?
        .map(|entry| {
            entry
                .map(|entry| entry.path())
                .map_err(|error| error.to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    paths.sort();
    if paths.is_empty() {
        return Ok(empty_replay());
    }

    let mut roles: Option<DeploymentRoles> = None;
    let mut last_digest: Option<String> = None;
    let mut events: BTreeMap<String, DeploymentEvent> = BTreeMap::new();
    let mut suspensions: BTreeMap<String, (String, String)> = BTreeMap::new();
    for (offset, path) in paths.iter().enumerate() {
        let metadata = std::fs::symlink_metadata(path).map_err(|error| {
            format!(
                "failed to inspect deployment event {}: {error}",
                path.display()
            )
        })?;
        if metadata.file_type().is_symlink()
            || !metadata.is_file()
            || metadata.len() > MAX_EVENT_BYTES
        {
            return Err(format!(
                "deployment event must be a real JSON file no larger than 1 MiB: {}",
                path.display()
            ));
        }
        let bytes = std::fs::read(path).map_err(|error| {
            format!(
                "failed to read deployment event {}: {error}",
                path.display()
            )
        })?;
        let digest = sha256_bytes(&bytes);
        let event: DeploymentEvent = serde_json::from_slice(&bytes)
            .map_err(|error| format!("invalid deployment event {}: {error}", path.display()))?;
        validate_event(&event, offset as u64, last_digest.as_deref())?;
        let expected_name = format!("{:020}-{}.json", event.sequence, event.id);
        if path.file_name().and_then(|name| name.to_str()) != Some(expected_name.as_str()) {
            return Err(format!(
                "deployment event filename does not match its identity: {}",
                path.display()
            ));
        }
        if events.contains_key(&event.id) {
            return Err(format!("duplicate deployment event id: {}", event.id));
        }
        match &event.payload {
            DeploymentEventPayload::Bootstrap { roles: initial } => {
                if offset != 0 || roles.is_some() {
                    return Err("Bootstrap must be the first and only bootstrap event".to_owned());
                }
                validate_roles(initial)?;
                roles = Some(initial.clone());
            }
            DeploymentEventPayload::Promotion {
                profile,
                before,
                after,
                qualification,
            } => {
                validate_profile_name(profile)?;
                let current = roles
                    .as_ref()
                    .ok_or("deployment history has no Bootstrap")?;
                if current != before
                    || before.rollback_profile != after.rollback_profile
                    || after.daily_default != *profile
                    || before.daily_default == *profile
                {
                    return Err("Promotion role transition is inconsistent".to_owned());
                }
                validate_qualification_reference(qualification)?;
                if suspensions
                    .values()
                    .any(|(candidate, _)| candidate == profile)
                {
                    return Err(format!("Promotion targets suspended Profile {profile}"));
                }
                roles = Some(after.clone());
            }
            DeploymentEventPayload::Rollback {
                profile,
                promotion_event_id,
                before,
                after,
                ..
            } => {
                let current = roles
                    .as_ref()
                    .ok_or("deployment history has no Bootstrap")?;
                if current != before
                    || before.daily_default != *profile
                    || after.daily_default != before.rollback_profile
                    || after.rollback_profile != before.rollback_profile
                {
                    return Err("Rollback role transition is inconsistent".to_owned());
                }
                match events
                    .get(promotion_event_id)
                    .map(|referenced| &referenced.payload)
                {
                    Some(DeploymentEventPayload::Promotion {
                        profile: promoted, ..
                    }) if promoted == profile => {}
                    _ => return Err("Rollback does not reference a matching Promotion".to_owned()),
                }
                suspensions.insert(event.id.clone(), (profile.clone(), event.reason.clone()));
                roles = Some(after.clone());
            }
            DeploymentEventPayload::Incident {
                profile,
                promotion_event_id,
            } => {
                validate_profile_name(profile)?;
                if roles.is_none() {
                    return Err("deployment history has no Bootstrap".to_owned());
                }
                if let Some(promotion_event_id) = promotion_event_id {
                    match events
                        .get(promotion_event_id)
                        .map(|referenced| &referenced.payload)
                    {
                        Some(DeploymentEventPayload::Promotion {
                            profile: promoted, ..
                        }) if promoted == profile => {}
                        _ => {
                            return Err(
                                "Incident does not reference a matching Promotion".to_owned()
                            );
                        }
                    }
                }
                suspensions.insert(event.id.clone(), (profile.clone(), event.reason.clone()));
            }
            DeploymentEventPayload::IncidentResolution {
                profile,
                suspension_event_id,
            } => match suspensions.get(suspension_event_id) {
                Some((suspended_profile, _)) if suspended_profile == profile => {
                    suspensions.remove(suspension_event_id);
                }
                _ => {
                    return Err(
                        "Incident Resolution does not reference an open suspension".to_owned()
                    );
                }
            },
        }
        last_digest = Some(digest);
        events.insert(event.id.clone(), event);
    }
    let last = events.values().max_by_key(|event| event.sequence);
    let status = DeploymentStatus {
        schema: EVENT_SCHEMA,
        initialized: true,
        roles,
        last_sequence: last.map(|event| event.sequence),
        last_event_id: last.map(|event| event.id.clone()),
        event_count: events.len(),
        open_suspensions: suspensions
            .iter()
            .map(|(event_id, (profile, reason))| OpenSuspension {
                event_id: event_id.clone(),
                profile: profile.clone(),
                reason: reason.clone(),
            })
            .collect(),
    };
    Ok(Replay {
        status,
        last_digest,
        events,
    })
}

fn empty_replay() -> Replay {
    Replay {
        status: DeploymentStatus {
            schema: EVENT_SCHEMA,
            initialized: false,
            roles: None,
            last_sequence: None,
            last_event_id: None,
            event_count: 0,
            open_suspensions: Vec::new(),
        },
        last_digest: None,
        events: BTreeMap::new(),
    }
}

fn validate_qualification_reference(reference: &QualificationReference) -> Result<(), String> {
    if reference.final_run_id.trim().is_empty()
        || reference.tuning_run_ids.is_empty()
        || !is_sha256(&reference.profile_sha256)
    {
        return Err("Promotion Qualification reference is incomplete".to_owned());
    }
    let mut run_ids = BTreeSet::new();
    run_ids.insert(reference.final_run_id.as_str());
    for id in &reference.tuning_run_ids {
        if id.trim().is_empty() || !run_ids.insert(id.as_str()) {
            return Err(
                "Promotion Qualification run identities are empty or duplicated".to_owned(),
            );
        }
    }
    if [
        &reference.identity.hardware,
        &reference.identity.software,
        &reference.identity.model,
        &reference.identity.runtime,
        &reference.identity.workload,
        &reference.identity.configuration,
        &reference.identity.policy,
    ]
    .into_iter()
    .any(|value| !is_sha256(value))
    {
        return Err("Promotion Evidence Identity is incomplete or malformed".to_owned());
    }
    Ok(())
}

fn validate_event(
    event: &DeploymentEvent,
    sequence: u64,
    previous: Option<&str>,
) -> Result<(), String> {
    if event.schema != EVENT_SCHEMA || event.sequence != sequence {
        return Err("deployment event schema or sequence is invalid".to_owned());
    }
    validate_event_id(&event.id)?;
    validate_actor_and_reason(&event.operator, &event.reason)?;
    if !is_canonical_utc_timestamp(&event.created_at) {
        return Err("deployment event timestamp is invalid".to_owned());
    }
    if event.previous_event_sha256.as_deref() != previous {
        return Err("deployment event hash chain is invalid".to_owned());
    }
    Ok(())
}

fn required_roles(current: &Replay) -> Result<DeploymentRoles, String> {
    current
        .status
        .roles
        .clone()
        .ok_or_else(|| "deployment history is not initialized".to_owned())
}

fn deployment_lock(install_root: &Path, timeout: Duration) -> Result<InterprocessLock, String> {
    InterprocessLock::acquire(&install_root.join(".deployment.lock"), timeout)
}

fn event_directory(install_root: &Path) -> PathBuf {
    install_root.join("deployment").join("events")
}

fn ensure_profile_exists(install_root: &Path, profile: &str) -> Result<(), String> {
    let path = install_root
        .join("profiles")
        .join(format!("{profile}.json"));
    if !path.is_file() {
        return Err(format!(
            "deployment Profile is unavailable: {}",
            path.display()
        ));
    }
    Ok(())
}

fn validate_roles(roles: &DeploymentRoles) -> Result<(), String> {
    validate_profile_name(&roles.daily_default)?;
    validate_profile_name(&roles.rollback_profile)
}

fn validate_profile_name(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 100
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(
            "Profile name must contain lowercase ASCII letters, digits, or hyphens".to_owned(),
        );
    }
    Ok(())
}

fn validate_event_id(value: &str) -> Result<(), String> {
    if value.len() != 32 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("deployment event id must be 32 hexadecimal characters".to_owned());
    }
    Ok(())
}

fn validate_actor_and_reason(operator: &str, reason: &str) -> Result<(), String> {
    validate_text("operator", operator, 200)?;
    validate_text("reason", reason, 4000)
}

fn validate_text(label: &str, value: &str, maximum: usize) -> Result<(), String> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > maximum
        || value
            .chars()
            .any(|character| character.is_control() && character != '\n' && character != '\t')
    {
        return Err(format!(
            "deployment {label} is missing, too long, or contains unsafe controls"
        ));
    }
    Ok(())
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn is_canonical_utc_timestamp(value: &str) -> bool {
    static UTC: OnceLock<regex::Regex> = OnceLock::new();
    UTC.get_or_init(|| {
        regex::Regex::new(
            r"^[0-9]{4}-(0[1-9]|1[0-2])-(0[1-9]|[12][0-9]|3[01])T([01][0-9]|2[0-3]):[0-5][0-9]:[0-5][0-9](\.[0-9]{1,9})?Z$",
        )
        .expect("static deployment UTC timestamp regex")
    })
    .is_match(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(root: &Path, name: &str) {
        std::fs::create_dir_all(root.join("profiles")).unwrap();
        std::fs::write(root.join("profiles").join(format!("{name}.json")), b"{}\n").unwrap();
    }

    #[test]
    fn event_history_derives_roles_and_preserves_suspensions_append_only() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        profile(root, "stable-16k");
        profile(root, "turbo-16k");
        let bootstrap = bootstrap(&BootstrapDeploymentOptions {
            install_root: root.to_path_buf(),
            daily_default: "stable-16k".to_owned(),
            rollback_profile: "stable-16k".to_owned(),
            operator: "setup".to_owned(),
            reason: "initialize conservative deployment roles".to_owned(),
            lock_timeout: Duration::from_secs(1),
        })
        .unwrap();
        assert_eq!(bootstrap.status.roles.unwrap().daily_default, "stable-16k");

        let current = replay(root).unwrap();
        let promotion_id = uuid::Uuid::new_v4().simple().to_string();
        let promotion = DeploymentEventPayload::Promotion {
            profile: "turbo-16k".to_owned(),
            before: DeploymentRoles {
                daily_default: "stable-16k".to_owned(),
                rollback_profile: "stable-16k".to_owned(),
            },
            after: DeploymentRoles {
                daily_default: "turbo-16k".to_owned(),
                rollback_profile: "stable-16k".to_owned(),
            },
            qualification: Box::new(QualificationReference {
                final_run_id: "final".to_owned(),
                tuning_run_ids: vec!["tuning".to_owned()],
                identity: EvidenceIdentity {
                    hardware: "a".repeat(64),
                    software: "b".repeat(64),
                    model: "c".repeat(64),
                    runtime: "d".repeat(64),
                    workload: "e".repeat(64),
                    configuration: "f".repeat(64),
                    policy: "1".repeat(64),
                },
                profile_sha256: "2".repeat(64),
            }),
        };
        let promoted = append(
            root,
            &current,
            "operator",
            "approved deployment",
            promotion,
            None,
        )
        .unwrap();
        let actual_promotion_id = promoted.event.id.clone();
        assert_ne!(actual_promotion_id, promotion_id);
        assert_eq!(promoted.status.roles.unwrap().daily_default, "turbo-16k");

        let rolled_back = rollback(&RollbackDeploymentOptions {
            install_root: root.to_path_buf(),
            expected_daily_default: "turbo-16k".to_owned(),
            promotion_event_id: actual_promotion_id,
            disposition: RollbackDisposition::Suspended,
            operator: "operator".to_owned(),
            reason: "daily operation contradicted the qualification".to_owned(),
            lock_timeout: Duration::from_secs(1),
        })
        .unwrap();
        assert_eq!(
            rolled_back.status.roles.unwrap().daily_default,
            "stable-16k"
        );
        assert_eq!(rolled_back.status.open_suspensions.len(), 1);
        assert_eq!(rolled_back.status.event_count, 3);

        let suspension_id = rolled_back.status.open_suspensions[0].event_id.clone();
        let resolved = resolve_incident(&ResolveIncidentOptions {
            install_root: root.to_path_buf(),
            suspension_event_id: suspension_id,
            profile: "turbo-16k".to_owned(),
            operator: "operator".to_owned(),
            resolution: "policy was strengthened and new evidence is required".to_owned(),
            lock_timeout: Duration::from_secs(1),
        })
        .unwrap();
        assert!(resolved.status.open_suspensions.is_empty());
        assert_eq!(resolved.status.event_count, 4);
    }

    #[test]
    fn chained_history_tampering_fails_closed() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        profile(root, "stable-16k");
        bootstrap(&BootstrapDeploymentOptions {
            install_root: root.to_path_buf(),
            daily_default: "stable-16k".to_owned(),
            rollback_profile: "stable-16k".to_owned(),
            operator: "setup".to_owned(),
            reason: "initialize".to_owned(),
            lock_timeout: Duration::from_secs(1),
        })
        .unwrap();
        record_incident(&RecordIncidentOptions {
            install_root: root.to_path_buf(),
            profile: "stable-16k".to_owned(),
            promotion_event_id: None,
            operator: "operator".to_owned(),
            reason: "contradictory operational evidence".to_owned(),
            lock_timeout: Duration::from_secs(1),
        })
        .unwrap();
        let path = std::fs::read_dir(event_directory(root))
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .min()
            .unwrap();
        let mut event: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        event["reason"] = serde_json::json!("tampered");
        std::fs::write(&path, serde_json::to_vec_pretty(&event).unwrap()).unwrap();
        assert!(replay(root).is_err());
    }

    #[test]
    fn gapped_or_misnamed_history_fails_closed() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        profile(root, "stable-16k");
        bootstrap(&BootstrapDeploymentOptions {
            install_root: root.to_path_buf(),
            daily_default: "stable-16k".to_owned(),
            rollback_profile: "stable-16k".to_owned(),
            operator: "setup".to_owned(),
            reason: "initialize".to_owned(),
            lock_timeout: Duration::from_secs(1),
        })
        .unwrap();
        let original = std::fs::read_dir(event_directory(root))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let misnamed = event_directory(root).join(format!("{:020}-{}.json", 1, "f".repeat(32)));
        std::fs::rename(original, misnamed).unwrap();
        assert!(replay(root).is_err());
    }
}
