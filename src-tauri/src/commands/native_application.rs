#[tauri::command]
pub fn native_application_preview_mode() -> bool {
    crate::readonly_provider_preview()
}
