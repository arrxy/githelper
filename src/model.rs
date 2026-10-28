use crate::{commit_message::CommitMessage, plan::Proposal, repository::Context};
use serde::{Deserialize, Serialize};

pub struct AppleModel;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClarificationTurn {
    pub question: String,
    pub answer: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ModelStatus {
    pub available: bool,
    pub detail: String,
}

#[derive(Serialize)]
struct Request<'a> {
    mode: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    request: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    context: Option<&'a Context>,
    #[serde(skip_serializing_if = "Option::is_none")]
    history: Option<&'a [ClarificationTurn]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    staged_diff: Option<&'a str>,
}

#[derive(Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
enum Response {
    #[serde(rename = "plan")]
    Plan { plan: Proposal },
    #[serde(rename = "commit_message")]
    CommitMessage { commit_message: CommitMessage },
    #[serde(rename = "status")]
    Status { available: bool, detail: String },
    #[serde(rename = "error")]
    Error { error: String },
}

impl AppleModel {
    pub fn status(&self) -> Result<ModelStatus, Box<dyn std::error::Error>> {
        match invoke(&Request {
            mode: "doctor",
            request: None,
            context: None,
            history: None,
            staged_diff: None,
        })? {
            Response::Status { available, detail } => Ok(ModelStatus { available, detail }),
            _ => Err("unexpected model status response".into()),
        }
    }

    pub fn translate(
        &self,
        request: &str,
        context: &Context,
        history: &[ClarificationTurn],
    ) -> Result<Proposal, Box<dyn std::error::Error>> {
        match invoke(&Request {
            mode: "translate",
            request: Some(request),
            context: Some(context),
            history: Some(history),
            staged_diff: None,
        })? {
            Response::Plan { plan } => Ok(plan),
            _ => Err("unexpected model translation response".into()),
        }
    }

    pub fn draft_commit(
        &self,
        staged_diff: &str,
    ) -> Result<CommitMessage, Box<dyn std::error::Error>> {
        match invoke(&Request {
            mode: "draftCommit",
            request: None,
            context: None,
            history: None,
            staged_diff: Some(staged_diff),
        })? {
            Response::CommitMessage { commit_message } => commit_message.validate(),
            _ => Err("unexpected model commit message response".into()),
        }
    }
}

fn invoke(request: &Request<'_>) -> Result<Response, Box<dyn std::error::Error>> {
    match platform_invoke(request)? {
        Response::Error { error } => Err(annotate_setup_error(request.mode, error).into()),
        response => Ok(response),
    }
}

fn annotate_setup_error(mode: &str, error: String) -> String {
    if mode == "doctor" || !is_setup_error(&error) {
        error
    } else {
        crate::setup_text::with_doctor_hint(&error)
    }
}

fn is_setup_error(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    lower.contains("unavailable")
        || lower.contains("apple intelligence")
        || lower.contains("macos 26")
        || lower.contains("on-device model")
        || lower.contains("model bridge")
        || lower.contains("model helper")
}

/// Doctor prints its own fix hints. Other commands point at `--doctor`.
fn setup_failure(mode: &str, message: String) -> Box<dyn std::error::Error> {
    if mode == "doctor" {
        message.into()
    } else {
        crate::setup_text::with_doctor_hint(&message).into()
    }
}

fn platform_invoke(request: &Request<'_>) -> Result<Response, Box<dyn std::error::Error>> {
    // The stand-in is compiled only with `--features fake-model`. A default
    // build has no branch here, so `GITHELPER_FAKE_MODEL` is ignored and the
    // string is absent from the release binary.
    #[cfg(feature = "fake-model")]
    if let Some(response) = fake_model::from_env()? {
        return Ok(response);
    }

    #[cfg(not(target_os = "macos"))]
    {
        if request.mode == "doctor" {
            return Ok(Response::Status {
                available: false,
                detail: crate::setup_text::MODEL_UNSUPPORTED_PLATFORM.into(),
            });
        }
        Err(setup_failure(
            request.mode,
            crate::setup_text::MODEL_RUNTIME_UNSUPPORTED.into(),
        ))
    }
    #[cfg(target_os = "macos")]
    {
        use std::{
            io::Write,
            process::{Command, Stdio},
        };
        const HELPER: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/githelper-model"));
        let helper = helper_cache::cached_helper_path(HELPER, &helper_cache::user_cache_dir()?)?;
        let mut child = Command::new(helper)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| {
                setup_failure(
                    request.mode,
                    format!("Could not start the Apple model bridge ({error})."),
                )
            })?;
        let mut input = child.stdin.take().ok_or_else(|| {
            setup_failure(
                request.mode,
                "Could not open the Apple model bridge input.".into(),
            )
        })?;
        input.write_all(&serde_json::to_vec(request)?)?;
        drop(input);
        let output = child.wait_with_output().map_err(|error| {
            setup_failure(
                request.mode,
                format!("The Apple model bridge failed ({error})."),
            )
        })?;
        let response: Response = serde_json::from_slice(&output.stdout).map_err(|error| {
            setup_failure(
                request.mode,
                format!(
                    "Could not read the Apple model response ({error}); helper: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                ),
            )
        })?;
        if !output.status.success() {
            return Err(setup_failure(
                request.mode,
                "The Apple model bridge failed.".into(),
            ));
        }
        Ok(response)
    }
}

/// Test-only stand-in for the Apple model bridge.
///
/// Compiled only with `--features fake-model`, which is not a default feature.
/// `GITHELPER_FAKE_MODEL` is either one model response JSON object, or a path to
/// a UTF-8 file containing one. The value uses the Swift bridge schema (`kind`
/// of `plan`, `commit_message`, `status`, or `error`). Unset, this build behaves
/// like a build without the feature.
///
/// This path only reads and parses that JSON. It does not run Git. Callers still
/// run `Proposal::validate` or `CommitMessage::validate` on success responses.
#[cfg(feature = "fake-model")]
mod fake_model {
    use super::Response;

    const MAX_FIXTURE_BYTES: u64 = 1_000_000;

    pub(super) fn from_env() -> Result<Option<Response>, Box<dyn std::error::Error>> {
        match std::env::var_os("GITHELPER_FAKE_MODEL") {
            None => Ok(None),
            Some(value) => {
                let Some(value) = value.to_str() else {
                    return Err("GITHELPER_FAKE_MODEL is not valid Unicode".into());
                };
                Ok(Some(parse(value)?))
            }
        }
    }

    pub(super) fn parse(value: &str) -> Result<Response, Box<dyn std::error::Error>> {
        let value = value.trim();
        if value.is_empty() {
            return Err("GITHELPER_FAKE_MODEL is set but empty".into());
        }
        let json = if value.starts_with('{') {
            value.to_owned()
        } else {
            read_fixture(value)?
        };
        serde_json::from_str(json.trim()).map_err(|error| {
            format!("GITHELPER_FAKE_MODEL is not a model response: {error}").into()
        })
    }

    fn read_fixture(path: &str) -> Result<String, Box<dyn std::error::Error>> {
        let metadata = std::fs::metadata(path).map_err(|error| {
            format!("could not read GITHELPER_FAKE_MODEL fixture {path}: {error}")
        })?;
        if !metadata.is_file() {
            return Err(format!("GITHELPER_FAKE_MODEL fixture {path} is not a file").into());
        }
        if metadata.len() > MAX_FIXTURE_BYTES {
            return Err(format!(
                "GITHELPER_FAKE_MODEL fixture {path} exceeds {MAX_FIXTURE_BYTES} bytes"
            )
            .into());
        }
        let text = std::fs::read_to_string(path).map_err(|error| {
            format!("could not read GITHELPER_FAKE_MODEL fixture {path}: {error}")
        })?;
        if u64::try_from(text.len()).unwrap_or(u64::MAX) > MAX_FIXTURE_BYTES {
            return Err(format!(
                "GITHELPER_FAKE_MODEL fixture {path} exceeds {MAX_FIXTURE_BYTES} bytes"
            )
            .into());
        }
        Ok(text)
    }
}

#[cfg(any(test, target_os = "macos"))]
mod helper_cache {
    use std::{
        fs::{self, File, OpenOptions},
        io::{Read, Write},
        os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
        path::{Path, PathBuf},
        sync::atomic::{AtomicU64, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    /// Directory that holds one cached copy of each embedded helper build.
    #[cfg(target_os = "macos")]
    pub(super) fn user_cache_dir() -> Result<PathBuf, Box<dyn std::error::Error>> {
        let xdg = std::env::var_os("XDG_CACHE_HOME");
        let home = std::env::var_os("HOME");
        resolve_cache_dir(xdg.as_deref(), home.as_deref(), cfg!(target_os = "macos"))
            .map_err(Into::into)
    }

    pub(super) fn resolve_cache_dir(
        xdg_cache_home: Option<&std::ffi::OsStr>,
        home: Option<&std::ffi::OsStr>,
        macos_layout: bool,
    ) -> Result<PathBuf, &'static str> {
        if let Some(xdg) = xdg_cache_home.filter(|value| !value.is_empty()) {
            return Ok(Path::new(xdg).join("githelper"));
        }
        let home = match home {
            Some(home) if !home.is_empty() => home,
            _ => return Err("could not locate a user cache directory"),
        };
        let home = Path::new(home);
        Ok(if macos_layout {
            home.join("Library/Caches/githelper")
        } else {
            home.join(".cache/githelper")
        })
    }

    /// Return a private, verified path to `bytes`, installing it on first use.
    ///
    /// The file name is a content id, so a new build does not reuse an older
    /// helper. Reuse still requires the same owner, mode `0700`, and exact bytes.
    pub(super) fn cached_helper_path(
        bytes: &[u8],
        cache_dir: &Path,
    ) -> Result<PathBuf, Box<dyn std::error::Error>> {
        if bytes.is_empty() {
            return Err("embedded model helper is empty".into());
        }
        ensure_private_dir(cache_dir)?;
        let path = cache_dir.join(format!("githelper-model-{}", content_id(bytes)));
        for _ in 0..2 {
            if verified_helper(&path, bytes) {
                return Ok(path);
            }
            install_helper(&path, bytes)?;
        }
        if verified_helper(&path, bytes) {
            Ok(path)
        } else {
            Err("cached model helper failed verification".into())
        }
    }

    /// FNV-1a 128-bit. This only selects the cache path; it is not the integrity check.
    pub(super) fn content_id(bytes: &[u8]) -> String {
        let mut hash = 0x6c62272e07bb014262b821756295c58d_u128;
        const PRIME: u128 = 0x0000_0000_0100_0000_0000_0000_0000_013B;
        for byte in bytes {
            hash ^= u128::from(*byte);
            hash = hash.wrapping_mul(PRIME);
        }
        format!("{hash:032x}")
    }

    fn current_uid() -> u32 {
        // SAFETY: geteuid is the POSIX effective-uid getter. On Linux and macOS it
        // returns a 32-bit uid_t and has no preconditions.
        unsafe extern "C" {
            fn geteuid() -> u32;
        }
        // SAFETY: geteuid only reads the calling process's credential.
        unsafe { geteuid() }
    }

    fn ensure_private_dir(dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
        match fs::symlink_metadata(dir) {
            Ok(meta) => {
                if !meta.file_type().is_dir() {
                    return Err(
                        format!("model cache path {} is not a directory", dir.display()).into(),
                    );
                }
                if meta.uid() != current_uid() {
                    return Err(format!(
                        "model cache directory {} is not owned by the current user",
                        dir.display()
                    )
                    .into());
                }
                if meta.permissions().mode() & 0o7777 != 0o700 {
                    fs::set_permissions(dir, fs::Permissions::from_mode(0o700)).map_err(
                        |error| {
                            format!(
                                "could not restrict model cache directory {}: {error}",
                                dir.display()
                            )
                        },
                    )?;
                }
                Ok(())
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir_all(dir).map_err(|error| {
                    format!(
                        "could not create model cache directory {}: {error}",
                        dir.display()
                    )
                })?;
                fs::set_permissions(dir, fs::Permissions::from_mode(0o700)).map_err(|error| {
                    format!(
                        "could not restrict model cache directory {}: {error}",
                        dir.display()
                    )
                })?;
                let meta = fs::symlink_metadata(dir)?;
                if !meta.file_type().is_dir() || meta.uid() != current_uid() {
                    return Err(format!(
                        "model cache directory {} is not a private directory",
                        dir.display()
                    )
                    .into());
                }
                Ok(())
            }
            Err(error) => Err(format!(
                "could not inspect model cache directory {}: {error}",
                dir.display()
            )
            .into()),
        }
    }

    fn install_helper(path: &Path, bytes: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
        let parent = path.parent().ok_or("model cache path has no directory")?;
        ensure_private_dir(parent)?;
        // Retry on a rare temp-name collision rather than deleting another
        // installer's in-flight file (create_new EEXIST + remove_file).
        let mut temporary = None;
        let mut last_error = None;
        for _ in 0..8 {
            let candidate = parent.join(format!(
                ".githelper-model.{}.{}.tmp",
                std::process::id(),
                unique_suffix()
            ));
            match write_private_file(&candidate, bytes) {
                Ok(()) => {
                    temporary = Some(candidate);
                    break;
                }
                Err(error) => {
                    // Only remove our candidate if we created it; create_new
                    // failures leave any colliding path alone.
                    last_error = Some(error);
                }
            }
        }
        let temporary = temporary.ok_or_else(|| {
            last_error.unwrap_or_else(|| "could not create a private temp helper".into())
        })?;
        // A directory cannot be replaced by rename(2). Symlinks are replaced, not followed.
        if let Ok(meta) = fs::symlink_metadata(path)
            && meta.file_type().is_dir()
            && let Err(error) = fs::remove_dir(path)
        {
            let _ = fs::remove_file(&temporary);
            return Err(format!(
                "could not replace model cache directory {}: {error}",
                path.display()
            )
            .into());
        }
        if let Err(error) = fs::rename(&temporary, path) {
            let _ = fs::remove_file(&temporary);
            return Err(format!(
                "could not install model helper into {}: {error}",
                path.display()
            )
            .into());
        }
        if let Ok(directory) = File::open(parent) {
            let _ = directory.sync_all();
        }
        Ok(())
    }

    fn write_private_file(path: &Path, bytes: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o700)
            .open(path)
            .map_err(|error| format!("could not create {}: {error}", path.display()))?;
        // open(2) applies the umask, so set the mode explicitly before the file is visible
        // under its final name.
        file.set_permissions(fs::Permissions::from_mode(0o700))
            .map_err(|error| format!("could not restrict {}: {error}", path.display()))?;
        file.write_all(bytes)
            .map_err(|error| format!("could not write {}: {error}", path.display()))?;
        file.sync_all()
            .map_err(|error| format!("could not persist {}: {error}", path.display()))?;
        Ok(())
    }

    fn unique_suffix() -> u128 {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        // Counter and time live in separate bit ranges so XOR cannot cancel
        // and two threads cannot mint the same temp name.
        let tick = u128::from(COUNTER.fetch_add(1, Ordering::Relaxed));
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        (tick << 64) ^ (nanos & 0xffff_ffff_ffff_ffff) ^ (u128::from(std::process::id()) << 48)
    }

    // Linux includes O_NOFOLLOW as 0x20000. macOS uses 0x0100.
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    const O_NOFOLLOW: i32 = 0x0100;
    #[cfg(not(any(target_os = "macos", target_os = "ios")))]
    const O_NOFOLLOW: i32 = 0x20000;

    pub(super) fn verified_helper(path: &Path, bytes: &[u8]) -> bool {
        // Open the inode we will judge. Concurrent installers publish a new
        // inode with rename, so a path stat and a later open can name different
        // files that are both valid copies. O_NOFOLLOW keeps a swapped symlink
        // from being accepted in that window.
        let Ok(file) = OpenOptions::new()
            .read(true)
            .custom_flags(O_NOFOLLOW)
            .open(path)
        else {
            return false;
        };
        let Ok(meta) = file.metadata() else {
            return false;
        };
        if !meta.file_type().is_file() || meta.uid() != current_uid() {
            return false;
        }
        if meta.permissions().mode() & 0o7777 != 0o700 {
            return false;
        }
        if u64::try_from(bytes.len()).ok() != Some(meta.len()) {
            return false;
        }
        contents_match(&file, bytes)
    }

    fn contents_match(file: &File, expected: &[u8]) -> bool {
        let limit = u64::try_from(expected.len()).unwrap_or(u64::MAX);
        let mut found = Vec::new();
        file.take(limit.saturating_add(1))
            .read_to_end(&mut found)
            .is_ok()
            && found == expected
    }
}

#[cfg(test)]
mod tests {
    use super::helper_cache::{cached_helper_path, content_id, resolve_cache_dir, verified_helper};
    use super::*;
    use std::{
        fs,
        os::unix::fs::{MetadataExt, PermissionsExt},
        path::PathBuf,
    };

    #[test]
    fn draft_request_contains_only_staged_patch() {
        let request = Request {
            mode: "draftCommit",
            request: None,
            context: None,
            history: None,
            staged_diff: Some("diff --git a/a b/a\n+hello"),
        };
        assert_eq!(
            serde_json::to_value(request).unwrap(),
            serde_json::json!({
                "mode": "draftCommit", "staged_diff": "diff --git a/a b/a\n+hello"
            })
        );
    }

    #[test]
    fn translation_replays_typed_history_and_original_request() {
        let history = [ClarificationTurn {
            question: "Message?".into(),
            answer: "Fix docs".into(),
        }];
        let request = Request {
            mode: "translate",
            request: Some("commit my changes"),
            context: None,
            history: Some(&history),
            staged_diff: None,
        };
        let value = serde_json::to_value(request).unwrap();
        assert_eq!(value["request"], "commit my changes");
        assert_eq!(
            value["history"][0],
            serde_json::json!({"question":"Message?","answer":"Fix docs"})
        );
        assert!(value.get("staged_diff").is_none());
    }

    #[test]
    fn setup_failures_point_at_doctor_outside_doctor_mode() {
        let runtime = setup_failure(
            "translate",
            crate::setup_text::MODEL_RUNTIME_UNSUPPORTED.into(),
        );
        let text = runtime.to_string();
        assert!(text.contains("githelper --doctor"));
        assert!(text.contains("macOS 26"));
        assert!(text.contains("Apple Intelligence"));
        let during_doctor = setup_failure(
            "doctor",
            "Could not start the Apple model bridge (missing).".into(),
        );
        assert!(!during_doctor.to_string().contains("githelper --doctor"));
        let unavailable = annotate_setup_error(
            "translate",
            "Apple model unavailable: deviceNotEligible.".into(),
        );
        assert!(unavailable.contains("githelper --doctor"));
        let already = annotate_setup_error("translate", unavailable.clone());
        assert_eq!(already, unavailable);
        let programming = annotate_setup_error("translate", "Invalid translation request.".into());
        assert!(!programming.contains("githelper --doctor"));
    }

    #[test]
    fn response_schema_rejects_missing_mixed_and_unknown_payloads() {
        for fixture in [
            r#"{"kind":"commit_message"}"#,
            r#"{"kind":"commit_message","commit_message":{"subject":"Fix docs"},"plan":{}}"#,
            r#"{"kind":"unknown"}"#,
            r#"{"kind":"status","available":true}"#,
        ] {
            assert!(
                serde_json::from_str::<Response>(fixture).is_err(),
                "{fixture}"
            );
        }
        let response: Response = serde_json::from_str(
            r#"{"kind":"commit_message","commit_message":{"subject":"Fix docs"}}"#,
        )
        .unwrap();
        match response {
            Response::CommitMessage { commit_message } => {
                assert_eq!(commit_message.validate().unwrap().subject, "Fix docs")
            }
            _ => panic!("wrong response"),
        }
    }

    #[test]
    fn content_id_is_stable_fnv_1a_128() {
        assert_eq!(content_id(b""), "6c62272e07bb014262b821756295c58d");
        assert_eq!(content_id(b"a"), "d228cb696f1a8caf78912b704e4a8964");
        assert_eq!(content_id(b"githelper"), "0605dd5db2060b4c8d313d7c90ac73bf");
        assert_ne!(content_id(b"a"), content_id(b"b"));
    }

    #[test]
    fn cache_dir_uses_xdg_then_platform_home() {
        use std::ffi::OsStr;
        assert_eq!(
            resolve_cache_dir(
                Some(OsStr::new("/var/cache")),
                Some(OsStr::new("/home/me")),
                false
            )
            .unwrap(),
            PathBuf::from("/var/cache/githelper")
        );
        assert_eq!(
            resolve_cache_dir(Some(OsStr::new("")), Some(OsStr::new("/home/me")), true).unwrap(),
            PathBuf::from("/home/me/Library/Caches/githelper")
        );
        assert_eq!(
            resolve_cache_dir(None, Some(OsStr::new("/home/me")), false).unwrap(),
            PathBuf::from("/home/me/.cache/githelper")
        );
        assert_eq!(
            resolve_cache_dir(None, Some(OsStr::new("/Users/me")), true).unwrap(),
            PathBuf::from("/Users/me/Library/Caches/githelper")
        );
        assert!(resolve_cache_dir(None, None, false).is_err());
        assert!(resolve_cache_dir(Some(OsStr::new("")), Some(OsStr::new("")), true).is_err());
    }

    fn mode(path: &std::path::Path) -> u32 {
        fs::symlink_metadata(path).unwrap().permissions().mode() & 0o7777
    }

    #[test]
    fn helper_is_installed_once_and_reused_only_when_it_matches() {
        let root = tempfile::tempdir().unwrap();
        let cache = root.path().join("cache");
        fs::create_dir(&cache).unwrap();
        fs::set_permissions(&cache, fs::Permissions::from_mode(0o755)).unwrap();
        let bytes = b"githelper-model-v1";
        let path = cached_helper_path(bytes, &cache).unwrap();
        assert_eq!(
            path.file_name().unwrap().to_str().unwrap(),
            format!("githelper-model-{}", content_id(bytes))
        );
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert_eq!(mode(&path), 0o700);
        assert_eq!(mode(&cache), 0o700);
        let inode = fs::metadata(&path).unwrap().ino();
        assert_eq!(cached_helper_path(bytes, &cache).unwrap(), path);
        assert_eq!(fs::metadata(&path).unwrap().ino(), inode);

        let other = cached_helper_path(b"githelper-model-v2", &cache).unwrap();
        assert_ne!(other, path);
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert_eq!(fs::read(&other).unwrap(), b"githelper-model-v2");

        fs::write(&path, b"tampered").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        let replaced = cached_helper_path(bytes, &cache).unwrap();
        assert_eq!(replaced, path);
        assert_ne!(fs::metadata(&path).unwrap().ino(), inode);
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert_eq!(mode(&path), 0o700);

        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        let inode = fs::metadata(&path).unwrap().ino();
        cached_helper_path(bytes, &cache).unwrap();
        assert_ne!(fs::metadata(&path).unwrap().ino(), inode);
        assert_eq!(mode(&path), 0o700);
        assert_eq!(fs::read(&path).unwrap(), bytes);

        let names: Vec<_> = fs::read_dir(&cache)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert!(
            names
                .iter()
                .all(|name| !name.to_string_lossy().ends_with(".tmp"))
        );
    }

    #[test]
    fn symlink_cache_entries_are_not_followed() {
        let root = tempfile::tempdir().unwrap();
        let target_dir = root.path().join("target");
        fs::create_dir(&target_dir).unwrap();
        let link = root.path().join("link");
        std::os::unix::fs::symlink(&target_dir, &link).unwrap();
        let error = cached_helper_path(b"helper", &link).unwrap_err();
        assert!(error.to_string().contains("not a directory"), "{error}");
        assert!(fs::read_dir(&target_dir).unwrap().next().is_none());

        let cache = root.path().join("cache");
        fs::create_dir(&cache).unwrap();
        let planted = root.path().join("planted");
        fs::write(&planted, b"do-not-touch").unwrap();
        let bytes = b"real-helper";
        let path = cache.join(format!("githelper-model-{}", content_id(bytes)));
        std::os::unix::fs::symlink(&planted, &path).unwrap();
        let installed = cached_helper_path(bytes, &cache).unwrap();
        assert_eq!(installed, path);
        assert!(
            fs::symlink_metadata(&installed)
                .unwrap()
                .file_type()
                .is_file()
        );
        assert_eq!(fs::read(&installed).unwrap(), bytes);
        assert_eq!(fs::read(&planted).unwrap(), b"do-not-touch");
        assert_eq!(mode(&installed), 0o700);

        // A symlink whose target already matches the helper must still be replaced.
        fs::remove_file(&installed).unwrap();
        fs::write(&planted, bytes).unwrap();
        fs::set_permissions(&planted, fs::Permissions::from_mode(0o700)).unwrap();
        let planted_inode = fs::metadata(&planted).unwrap().ino();
        std::os::unix::fs::symlink(&planted, &path).unwrap();
        let installed = cached_helper_path(bytes, &cache).unwrap();
        assert!(
            fs::symlink_metadata(&installed)
                .unwrap()
                .file_type()
                .is_file()
        );
        assert_ne!(fs::metadata(&installed).unwrap().ino(), planted_inode);
        assert_eq!(fs::read(&installed).unwrap(), bytes);
        assert_eq!(fs::read(&planted).unwrap(), bytes);
        assert_eq!(mode(&planted), 0o700);
    }

    #[test]
    fn empty_helper_and_non_directory_cache_are_rejected() {
        let root = tempfile::tempdir().unwrap();
        assert!(cached_helper_path(b"", root.path()).is_err());
        let file = root.path().join("not-a-directory");
        fs::write(&file, b"x").unwrap();
        assert!(cached_helper_path(b"helper", &file).is_err());
    }

    #[test]
    fn concurrent_installs_leave_one_verified_helper() {
        let root = tempfile::tempdir().unwrap();
        let cache = root.path().join("cache");
        let bytes = b"same-helper-bytes";
        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| {
                    let path = cached_helper_path(bytes, &cache).unwrap();
                    assert!(verified_helper(&path, bytes));
                    assert_eq!(mode(&path), 0o700);
                });
            }
        });
        let helpers: Vec<_> = fs::read_dir(&cache)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .filter(|name| name.to_string_lossy().starts_with("githelper-model-"))
            .collect();
        assert_eq!(helpers.len(), 1);
    }

    #[cfg(feature = "fake-model")]
    #[test]
    fn fake_fixture_parses_a_model_response_and_still_validates() {
        let response = super::fake_model::parse(
            r#"{"kind":"plan","plan":{"outcome":"ready","summary":"Show status","clarification":"","steps":[{"action":"status","value":"","paths":[]}]}}"#,
        )
        .unwrap();
        match response {
            super::Response::Plan { plan } => {
                let validated = plan.validate(std::path::Path::new(".")).unwrap();
                assert_eq!(
                    validated.commands[0].args,
                    ["status", "--short", "--branch"].map(str::to_owned)
                );
                assert!(!validated.commands[0].destructive);
                assert!(validated.commands[0].warning.is_none());
            }
            _ => panic!("expected a plan"),
        }

        let status = super::fake_model::parse(
            r#"{"kind":"status","available":false,"detail":"fixture status"}"#,
        )
        .unwrap();
        match status {
            super::Response::Status { available, detail } => {
                assert!(!available);
                assert_eq!(detail, "fixture status");
            }
            _ => panic!("expected status"),
        }

        for fixture in [
            "",
            "   ",
            "{",
            r#"{"kind":"plan","plan":{"outcome":"ready","summary":"Reset","clarification":"","steps":[{"action":"reset","value":"--hard","paths":[]}]}}"#,
            r#"{"kind":"plan","plan":{"outcome":"ready","summary":"Push","clarification":"","steps":[{"action":"push","value":"","paths":[],"destructive":true}]}}"#,
            r#"{"kind":"plan","commands":["git","reset","--hard"]}"#,
        ] {
            assert!(
                super::fake_model::parse(fixture).is_err(),
                "accepted {fixture:?}"
            );
        }
    }

    #[cfg(feature = "fake-model")]
    #[test]
    fn fake_fixture_file_is_read_and_oversized_files_are_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("response.json");
        std::fs::write(
            &path,
            "\n{\"kind\":\"commit_message\",\"commit_message\":{\"subject\":\"Fix ratio\"}}\n",
        )
        .unwrap();
        let response = super::fake_model::parse(path.to_str().unwrap()).unwrap();
        match response {
            super::Response::CommitMessage { commit_message } => {
                assert_eq!(commit_message.validate().unwrap().subject, "Fix ratio");
            }
            _ => panic!("expected a commit message"),
        }

        let huge = directory.path().join("huge.json");
        std::fs::write(&huge, vec![b' '; 1_000_001]).unwrap();
        match super::fake_model::parse(huge.to_str().unwrap()) {
            Err(error) => assert!(error.to_string().contains("exceeds"), "{error}"),
            Ok(_) => panic!("oversized fixture was accepted"),
        }
        match super::fake_model::parse(directory.path().to_str().unwrap()) {
            Err(error) => assert!(error.to_string().contains("not a file"), "{error}"),
            Ok(_) => panic!("directory fixture was accepted"),
        }
    }
}
