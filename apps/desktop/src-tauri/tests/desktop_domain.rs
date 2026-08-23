use alpine_desktop::assessment::{FitStatus, HardwareCapacity, assess_model};
use alpine_desktop::catalog::{
    decode_hugging_face_models, hydrate_model_artifacts, validated_artifact_filename,
    validated_remote_artifact_path,
};

#[test]
fn hugging_face_search_exposes_exact_gguf_artifacts() {
    let body = r#"[
      {
        "id": "Qwen/Qwen3.5-9B-GGUF",
        "author": "Qwen",
        "downloads": 42000,
        "likes": 900,
        "lastModified": "2026-08-20T10:00:00.000Z",
        "gated": false,
        "siblings": [
          {"rfilename": "Qwen3.5-9B-Q4_K_M.gguf", "size": 6123456789, "lfs": {"size": 6123456789, "oid": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}},
          {"rfilename": "Qwen3.5-9B-Q8_0.gguf", "size": 9876543210},
          {"rfilename": "README.md", "size": 12000}
        ]
      }
    ]"#;

    let models = decode_hugging_face_models(body).expect("fixture should decode");

    assert_eq!(models.len(), 1);
    assert_eq!(models[0].id, "Qwen/Qwen3.5-9B-GGUF");
    assert_eq!(models[0].publisher, "Qwen");
    assert_eq!(models[0].artifacts.len(), 2);
    assert_eq!(models[0].artifacts[0].filename, "Qwen3.5-9B-Q4_K_M.gguf");
    assert_eq!(models[0].artifacts[0].size_bytes, 6_123_456_789);
    assert_eq!(
        models[0].artifacts[0].sha256.as_deref(),
        Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
    );
    assert_eq!(
        models[0].artifacts[0].download_url,
        "https://huggingface.co/Qwen/Qwen3.5-9B-GGUF/resolve/main/Qwen3.5-9B-Q4_K_M.gguf"
    );
}

#[test]
fn download_targets_reject_path_traversal_and_non_gguf_files() {
    assert_eq!(
        validated_artifact_filename("Qwen-Q4_K_M.gguf").unwrap(),
        "Qwen-Q4_K_M.gguf"
    );
    assert!(validated_artifact_filename("../Qwen.gguf").is_err());
    assert!(validated_artifact_filename("nested/Qwen.gguf").is_err());
    assert!(validated_artifact_filename("README.md").is_err());
    assert_eq!(
        validated_remote_artifact_path("nested/Qwen.gguf").unwrap(),
        "nested/Qwen.gguf"
    );
    assert!(validated_remote_artifact_path("../Qwen.gguf").is_err());
    assert!(validated_remote_artifact_path("nested\\Qwen.gguf").is_err());
}

#[test]
fn hugging_face_tree_hydrates_sizes_missing_from_search_results() {
    let search = r#"[{"id":"Qwen/Qwen3.5-9B-GGUF","siblings":[{"rfilename":"Qwen-BF16.gguf"},{"rfilename":"nested/Qwen-Q4_K_M.gguf"}]}]"#;
    let tree = r#"[
      {"type":"file","path":"Qwen-BF16.gguf","size":24000000000},
      {"type":"file","path":"nested/Qwen-Q4_K_M.gguf","size":6123456789,"lfs":{"size":6123456789,"oid":"sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}},
      {"type":"file","path":"README.md","size":12000}
    ]"#;
    let mut models = decode_hugging_face_models(search).expect("search should decode");

    assert_eq!(models[0].artifacts[0].filename, "Qwen-BF16.gguf");
    hydrate_model_artifacts(&mut models[0], tree).expect("tree should hydrate");
    assert_eq!(models[0].artifacts[0].filename, "nested/Qwen-Q4_K_M.gguf");
    assert_eq!(models[0].artifacts[0].size_bytes, 6_123_456_789);
    assert_eq!(
        models[0].artifacts[0].sha256.as_deref(),
        Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
    );
    assert!(
        models[0].artifacts[0]
            .download_url
            .ends_with("nested/Qwen-Q4_K_M.gguf")
    );
}

#[test]
fn model_fit_keeps_estimates_separate_from_qualification() {
    let hardware = HardwareCapacity {
        total_memory_bytes: 68_719_476_736,
        dedicated_vram_bytes: 17_179_869_184,
    };

    let assessment = assess_model(&hardware, 12_073_953_824);

    assert_eq!(assessment.status, FitStatus::FitsGpuWithHeadroom);
    assert_eq!(assessment.estimated_runtime_bytes, 13_958_643_712);
    assert_eq!(assessment.headroom_bytes, 3_221_225_472);
    assert!(!assessment.is_measured);
    assert_eq!(
        assessment.evidence_label,
        "Estimate — run analysis to measure"
    );
}
