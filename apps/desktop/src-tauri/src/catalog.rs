use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const PATH_SEGMENT: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'%')
    .add(b'<')
    .add(b'>')
    .add(b'?')
    .add(b'`')
    .add(b'{')
    .add(b'}');

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelArtifact {
    pub filename: String,
    pub size_bytes: u64,
    pub sha256: Option<String>,
    pub download_url: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelSearchResult {
    pub id: String,
    pub revision: Option<String>,
    pub publisher: String,
    pub downloads: u64,
    pub likes: u64,
    pub last_modified: Option<String>,
    pub gated: bool,
    pub artifacts: Vec<ModelArtifact>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HubModel {
    id: String,
    #[serde(default)]
    sha: Option<String>,
    #[serde(default)]
    author: Option<String>,
    #[serde(default)]
    downloads: u64,
    #[serde(default)]
    likes: u64,
    #[serde(default)]
    last_modified: Option<String>,
    #[serde(default)]
    gated: serde_json::Value,
    #[serde(default)]
    siblings: Vec<HubSibling>,
}

#[derive(Debug, Deserialize)]
struct HubSibling {
    rfilename: String,
    #[serde(default)]
    size: Option<u64>,
    #[serde(default)]
    lfs: Option<HubLfs>,
}

#[derive(Debug, Deserialize)]
struct HubLfs {
    size: u64,
    #[serde(default)]
    oid: Option<String>,
}

#[derive(Debug, Deserialize)]
struct HubTreeEntry {
    #[serde(rename = "type")]
    entry_type: String,
    path: String,
    #[serde(default)]
    size: Option<u64>,
    #[serde(default)]
    lfs: Option<HubLfs>,
}

pub fn download_url(repo_id: &str, revision: &str, filename: &str) -> String {
    let encoded_filename = filename
        .split('/')
        .map(|segment| utf8_percent_encode(segment, PATH_SEGMENT).to_string())
        .collect::<Vec<_>>()
        .join("/");
    format!("https://huggingface.co/{repo_id}/resolve/{revision}/{encoded_filename}")
}

pub fn validated_revision(value: &str) -> Result<&str, String> {
    let value = value.trim();
    (value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then_some(value)
        .ok_or_else(|| {
            "model downloads require the exact 40-character Hugging Face revision".to_owned()
        })
}

fn lfs_sha256(lfs: &HubLfs) -> Option<String> {
    let value = lfs.oid.as_deref()?.strip_prefix("sha256:")?;
    (value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| value.to_ascii_lowercase())
}

pub fn validated_artifact_filename(value: &str) -> Result<&str, String> {
    let path = std::path::Path::new(value);
    let is_single_component = path
        .file_name()
        .is_some_and(|filename| filename == std::ffi::OsStr::new(value));
    if value.is_empty()
        || value.len() > 255
        || !is_single_component
        || !value.to_ascii_lowercase().ends_with(".gguf")
    {
        return Err("model downloads require a single safe GGUF filename".to_owned());
    }
    Ok(value)
}

pub fn validated_remote_artifact_path(value: &str) -> Result<&str, String> {
    let valid = !value.is_empty()
        && value.len() <= 512
        && !value.contains('\\')
        && value.to_ascii_lowercase().ends_with(".gguf")
        && value
            .split('/')
            .all(|segment| !segment.is_empty() && segment != "." && segment != "..");
    valid
        .then_some(value)
        .ok_or_else(|| "the remote GGUF artifact path is invalid".to_owned())
}

pub fn decode_hugging_face_models(body: &str) -> Result<Vec<ModelSearchResult>, String> {
    let models: Vec<HubModel> = serde_json::from_str(body)
        .map_err(|error| format!("Hugging Face returned invalid model metadata: {error}"))?;

    Ok(models
        .into_iter()
        .filter_map(|model| {
            let publisher = model
                .author
                .clone()
                .unwrap_or_else(|| model.id.split('/').next().unwrap_or("unknown").to_owned());
            let artifacts = model
                .siblings
                .into_iter()
                .filter(|file| file.rfilename.to_ascii_lowercase().ends_with(".gguf"))
                .map(|file| {
                    let size_bytes = file
                        .size
                        .or_else(|| file.lfs.as_ref().map(|lfs| lfs.size))
                        .unwrap_or(0);
                    let sha256 = file.lfs.as_ref().and_then(lfs_sha256);
                    ModelArtifact {
                        download_url: download_url(
                            &model.id,
                            model.sha.as_deref().unwrap_or("main"),
                            &file.rfilename,
                        ),
                        filename: file.rfilename,
                        size_bytes,
                        sha256,
                    }
                })
                .collect::<Vec<_>>();

            (!artifacts.is_empty()).then(|| ModelSearchResult {
                id: model.id,
                revision: model.sha,
                publisher,
                downloads: model.downloads,
                likes: model.likes,
                last_modified: model.last_modified,
                gated: match model.gated {
                    serde_json::Value::Bool(value) => value,
                    serde_json::Value::String(value) => value != "false" && !value.is_empty(),
                    _ => false,
                },
                artifacts,
            })
        })
        .collect())
}

pub fn hydrate_model_artifacts(model: &mut ModelSearchResult, body: &str) -> Result<(), String> {
    let entries: Vec<HubTreeEntry> = serde_json::from_str(body)
        .map_err(|error| format!("Hugging Face returned an invalid repository tree: {error}"))?;
    let metadata = entries
        .into_iter()
        .filter(|entry| entry.entry_type == "file")
        .filter_map(|entry| {
            let size = entry
                .size
                .or_else(|| entry.lfs.as_ref().map(|lfs| lfs.size))?;
            let sha256 = entry.lfs.as_ref().and_then(lfs_sha256);
            Some((entry.path, (size, sha256)))
        })
        .collect::<BTreeMap<_, _>>();

    for artifact in &mut model.artifacts {
        if let Some((size, sha256)) = metadata.get(&artifact.filename) {
            artifact.size_bytes = *size;
            artifact.sha256.clone_from(sha256);
            artifact.download_url = download_url(
                &model.id,
                model.revision.as_deref().unwrap_or("main"),
                &artifact.filename,
            );
        }
    }
    model.artifacts.sort_by_key(|artifact| {
        let name = artifact.filename.to_ascii_uppercase();
        let priority = if name.contains("Q4_K_M") {
            0
        } else if name.contains("Q5_K_M") {
            1
        } else if name.contains("Q4_K_S") {
            2
        } else if name.contains("Q8_0") {
            3
        } else if name.contains("BF16") || name.contains("F16") {
            20
        } else {
            10
        };
        (priority, artifact.size_bytes, name)
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{decode_hugging_face_models, validated_revision};

    #[test]
    fn search_retains_exact_hugging_face_revision() {
        let revision = "0123456789abcdef0123456789abcdef01234567";
        let body = format!(
            r#"[{{"id":"Qwen/Qwen-GGUF","sha":"{revision}","siblings":[{{"rfilename":"model.gguf","size":12}}]}}]"#
        );
        let models = decode_hugging_face_models(&body).unwrap();

        assert_eq!(models[0].revision.as_deref(), Some(revision));
        assert!(models[0].artifacts[0].download_url.contains(revision));
        assert_eq!(validated_revision(revision).unwrap(), revision);
        assert!(validated_revision("main").is_err());
    }
}
