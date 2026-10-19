//! Board persistence: CRDT snapshots into local-first storage (OPFS on web, the platform
//! cache dir on native) and JSON export. All filesystem/browser IO lives in this shell
//! module; the core never touches it.

#[cfg(not(target_arch = "wasm32"))]
pub mod native {
    use std::path::{Path, PathBuf};

    /// Where the autosaved board snapshot lives. Falls back to the temp dir when the
    /// platform has no cache dir; callers log the resolved path.
    pub fn board_path() -> PathBuf {
        let mut path = dirs::cache_dir().unwrap_or_else(std::env::temp_dir);
        path.push("e6paint");
        path.push("board.e6p");
        path
    }

    /// Where JSON exports land (next to the snapshot).
    pub fn json_path() -> PathBuf {
        let mut path = board_path();
        path.set_extension("json");
        path
    }

    /// Where SVG exports land (next to the snapshot).
    pub fn svg_path() -> PathBuf {
        let mut path = board_path();
        path.set_extension("svg");
        path
    }

    pub fn save(path: &Path, bytes: &[u8]) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
        std::fs::write(path, bytes).map_err(|err| err.to_string())
    }

    /// `Ok(None)` when nothing has been saved yet (first run) — that is expected, not an
    /// error; every other read failure surfaces.
    pub fn load(path: &Path) -> Result<Option<Vec<u8>>, String> {
        match std::fs::read(path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(err.to_string()),
        }
    }

    pub fn save_text(path: &Path, text: &str) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
        std::fs::write(path, text).map_err(|err| err.to_string())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn temp_path(name: &str) -> PathBuf {
            let mut path = std::env::temp_dir();
            path.push(format!("e6paint-test-{}-{}", std::process::id(), name));
            path
        }

        #[test]
        fn save_load_round_trip_and_first_run_semantics() -> Result<(), Box<dyn std::error::Error>>
        {
            let path = temp_path("roundtrip.e6p");
            assert!(
                matches!(load(&path), Ok(None)),
                "unsaved path reads as first run"
            );
            save(&path, b"e6snapshot")?;
            assert_eq!(load(&path)?, Some(b"e6snapshot".to_vec()));
            std::fs::remove_file(&path)?;
            Ok(())
        }

        #[test]
        fn save_creates_missing_directories() -> Result<(), Box<dyn std::error::Error>> {
            let path = temp_path("nested").join("deep").join("board.e6p");
            save(&path, b"x")?;
            assert_eq!(load(&path)?, Some(b"x".to_vec()));
            let mut ancestors = path.ancestors();
            ancestors.next();
            ancestors.next();
            std::fs::remove_dir_all(ancestors.next().ok_or("missing ancestor")?)?;
            Ok(())
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub mod web {
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;

    const BOARD_FILE: &str = "board.e6p";

    fn describe_js_error(what: &str, value: wasm_bindgen::JsValue) -> String {
        match value.dyn_ref::<js_sys::Error>() {
            Some(err) => format!("{what}: {} {}", err.name(), err.message()),
            None => format!("{what}: {value:?}"),
        }
    }

    async fn board_file_handle(create: bool) -> Result<web_sys::FileSystemFileHandle, String> {
        let window = web_sys::window().ok_or("no window")?;
        let storage = window.navigator().storage();
        let root = JsFuture::from(storage.get_directory())
            .await
            .map_err(|err| describe_js_error("OPFS get_directory", err))?;
        let root: web_sys::FileSystemDirectoryHandle = root
            .dyn_into()
            .map_err(|_| "OPFS get_directory did not yield a directory handle".to_string())?;
        let options = web_sys::FileSystemGetFileOptions::new();
        options.set_create(create);
        let handle = JsFuture::from(root.get_file_handle_with_options(BOARD_FILE, &options))
            .await
            .map_err(|err| describe_js_error(BOARD_FILE, err))?;
        handle
            .dyn_into()
            .map_err(|_| format!("{BOARD_FILE} did not yield a file handle"))
    }

    /// Autosave the CRDT snapshot into the origin-private filesystem.
    pub async fn save_snapshot(bytes: &[u8]) -> Result<(), String> {
        let handle = board_file_handle(true).await?;
        let writable = JsFuture::from(handle.create_writable())
            .await
            .map_err(|err| describe_js_error("create_writable", err))?;
        let writable: web_sys::FileSystemWritableFileStream = writable
            .dyn_into()
            .map_err(|_| "create_writable did not yield a writable stream".to_string())?;
        let write_promise = writable
            .write_with_u8_array(bytes)
            .map_err(|err| describe_js_error("write", err))?;
        JsFuture::from(write_promise)
            .await
            .map_err(|err| describe_js_error("write", err))?;
        JsFuture::from(writable.close())
            .await
            .map_err(|err| describe_js_error("close", err))?;
        Ok(())
    }

    /// Load the autosaved snapshot; `Ok(None)` when none exists yet (first run).
    pub async fn load_snapshot() -> Result<Option<Vec<u8>>, String> {
        let handle = match board_file_handle(false).await {
            Ok(handle) => handle,
            Err(err) if err.contains("NotFoundError") => return Ok(None),
            Err(err) => return Err(err),
        };
        let file = JsFuture::from(handle.get_file())
            .await
            .map_err(|err| describe_js_error("get_file", err))?;
        let file: web_sys::File = file
            .dyn_into()
            .map_err(|_| "get_file did not yield a File".to_string())?;
        let buffer = JsFuture::from(file.array_buffer())
            .await
            .map_err(|err| describe_js_error("array_buffer", err))?;
        let buffer: js_sys::ArrayBuffer = buffer
            .dyn_into()
            .map_err(|_| "array_buffer did not yield an ArrayBuffer".to_string())?;
        Ok(Some(js_sys::Uint8Array::new(&buffer).to_vec()))
    }

    /// Copy text to the clipboard (async under the hood; errors surfaced).
    pub fn copy_to_clipboard(text: &str) {
        let Some(window) = web_sys::window() else {
            log::error!("clipboard copy failed: no window");
            return;
        };
        let clipboard = window.navigator().clipboard();
        let promise = clipboard.write_text(text);
        wasm_bindgen_futures::spawn_local(async move {
            if let Err(err) = wasm_bindgen_futures::JsFuture::from(promise).await {
                log::error!("clipboard write failed: {err:?}");
            }
        });
    }

    /// Hand binary bytes to the user via the browser's download flow.
    pub fn download_bytes(filename: &str, mime: &str, bytes: &[u8]) -> Result<(), String> {
        let array = js_sys::Uint8Array::from(bytes);
        let parts = js_sys::Array::of1(&array.into());
        let bag = web_sys::BlobPropertyBag::new();
        bag.set_type(mime);
        let Some(blob) = web_sys::Blob::new_with_str_sequence_and_options(&parts.into(), &bag).ok()
        else {
            return Err("Blob build failed".to_string());
        };
        let Some(url) = web_sys::Url::create_object_url_with_blob(&blob).ok() else {
            return Err("object URL creation failed".to_string());
        };
        let Some(document) = web_sys::window().and_then(|window| window.document()) else {
            return Err("no document; download unavailable".to_string());
        };
        let Some(anchor) = document
            .create_element("a")
            .ok()
            .and_then(|element| element.dyn_into::<web_sys::HtmlAnchorElement>().ok())
        else {
            return Err("download anchor creation failed".to_string());
        };
        anchor.set_href(&url);
        anchor.set_download(filename);
        anchor.click();
        if let Err(err) = web_sys::Url::revoke_object_url(&url) {
            log::warn!("revoking the download URL failed: {err:?}");
        }
        Ok(())
    }

    /// Hand a text file to the user via the browser's download flow (delegates to
    /// [`download_bytes`] with a JSON mime type).
    pub fn download_text(filename: &str, text: &str) -> Result<(), String> {
        download_bytes(filename, "application/json", text.as_bytes())
    }
}
