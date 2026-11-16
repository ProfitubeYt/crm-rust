//! Native file dialogs. Each returns `None` when the user closed the dialog.

use std::path::PathBuf;

pub struct Picked {
    pub name: String,
    pub bytes: Vec<u8>,
}

/// What the person would have chosen in a dialog. A test must never open one.
#[cfg(test)]
pub static CHOSEN: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);

#[cfg(test)]
fn chosen() -> Option<PathBuf> {
    CHOSEN.lock().unwrap().clone()
}

#[cfg(not(test))]
async fn ask_where(name: &str) -> Option<PathBuf> {
    // The dialog offers the kind of workbook the name says it is: a macro
    // workbook saved as .xlsx would not open.
    let kind = std::path::Path::new(name).extension().and_then(|e| e.to_str()).unwrap_or("xlsx").to_owned();
    let target = rfd::AsyncFileDialog::new().set_file_name(name).add_filter("Книга Excel", &[kind.as_str()]).save_file().await?;
    Some(target.path().to_path_buf())
}

#[cfg(test)]
async fn ask_where(_name: &str) -> Option<PathBuf> {
    chosen()
}

/// Saves `bytes` where the user points and says where that was.
pub async fn save_as(name: String, bytes: Vec<u8>) -> Result<Option<String>, String> {
    let Some(path) = ask_where(&name).await else {
        return Ok(None);
    };
    tokio::fs::write(&path, bytes).await.map_err(|e| e.to_string())?;
    Ok(Some(path.display().to_string()))
}

#[cfg(not(test))]
pub async fn pick_path(extensions: &[&str]) -> Option<PathBuf> {
    let picked = rfd::AsyncFileDialog::new().add_filter("Книга Excel", extensions).pick_file().await?;
    Some(picked.path().to_path_buf())
}

#[cfg(test)]
pub async fn pick_path(_extensions: &[&str]) -> Option<PathBuf> {
    chosen()
}

pub async fn pick_file(extensions: &[&str]) -> Result<Option<Picked>, String> {
    let Some(path) = pick_path(extensions).await else { return Ok(None) };
    read(path).await.map(Some)
}

pub async fn read(path: PathBuf) -> Result<Picked, String> {
    let bytes = tokio::fs::read(&path).await.map_err(|e| e.to_string())?;
    let name = path.file_name().map_or_else(|| "book.xlsx".to_owned(), |n| n.to_string_lossy().into_owned());
    Ok(Picked { name, bytes })
}

#[cfg(not(test))]
async fn ask_which(kinds: &[(&str, &[&str])]) -> Option<PathBuf> {
    let dialog = kinds.iter().fold(rfd::AsyncFileDialog::new(), |dialog, (name, extensions)| dialog.add_filter(*name, extensions));
    Some(dialog.pick_file().await?.path().to_path_buf())
}

#[cfg(test)]
async fn ask_which(_kinds: &[(&str, &[&str])]) -> Option<PathBuf> {
    chosen()
}

/// A file of one of the kinds named, each a caption and its extensions.
pub async fn pick_of(kinds: &[(&str, &[&str])]) -> Result<Option<Picked>, String> {
    let Some(path) = ask_which(kinds).await else { return Ok(None) };
    read(path).await.map(Some)
}

#[cfg(not(test))]
async fn ask_where_any(name: &str) -> Option<PathBuf> {
    let dialog = rfd::AsyncFileDialog::new().set_file_name(name);
    let dialog = match std::path::Path::new(name).extension().and_then(|e| e.to_str()) {
        Some(kind) => dialog.add_filter(kind.to_uppercase(), &[kind]),
        None => dialog,
    };
    Some(dialog.save_file().await?.path().to_path_buf())
}

#[cfg(test)]
async fn ask_where_any(_name: &str) -> Option<PathBuf> {
    chosen()
}

/// [`save_as`] for a file that is not a workbook.
pub async fn save_any(name: String, bytes: Vec<u8>) -> Result<Option<String>, String> {
    let Some(path) = ask_where_any(&name).await else {
        return Ok(None);
    };
    tokio::fs::write(&path, bytes).await.map_err(|e| e.to_string())?;
    Ok(Some(path.display().to_string()))
}
