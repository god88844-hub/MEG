//! Undo only a mutation made through MEG's file tools and only if the file
//! still matches MEG's last output. Never overwrite subsequent manual edits.
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub struct FileCheckpoint {
    path: PathBuf,
    before: Option<Vec<u8>>,
}
impl FileCheckpoint {
    pub fn capture(workspace: &Path, args: &Value) -> Result<Self> {
        let path =
            crate::fs_tools::resolve(workspace, args["path"].as_str().context("Missing path")?)?;
        let before = if path.exists() {
            Some(std::fs::read(&path)?)
        } else {
            None
        };
        Ok(Self { path, before })
    }
    pub fn commit(self, workspace: &Path) -> Result<()> {
        let after = std::fs::read(&self.path)?;
        let dir = workspace.join(".meg").join("tui");
        std::fs::create_dir_all(&dir)?;
        let record =
            json!({"path": self.path.to_string_lossy(), "before": self.before, "after": after});
        std::fs::write(dir.join("undo.json"), serde_json::to_vec(&record)?)?;
        Ok(())
    }
}
pub fn undo(workspace: &Path) -> Result<String> {
    let journal = workspace.join(".meg").join("tui").join("undo.json");
    let record: Value = serde_json::from_slice(
        &std::fs::read(&journal).context("No recorded file mutation to undo")?,
    )?;
    let path = crate::fs_tools::resolve(
        workspace,
        record["path"].as_str().context("Invalid undo journal")?,
    )?;
    let after: Vec<u8> = serde_json::from_value(record["after"].clone())?;
    if std::fs::read(&path)? != after {
        bail!("File changed after MEG's edit; undo refused to preserve those changes");
    }
    let before: Option<Vec<u8>> = serde_json::from_value(record["before"].clone())?;
    if let Some(before) = before {
        std::fs::write(&path, before)?;
    } else {
        std::fs::remove_file(&path)?;
    }
    std::fs::remove_file(journal)?;
    Ok(format!(
        "Undid MEG's last file mutation: {}",
        path.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn undo_restores_recorded_edits_but_refuses_later_manual_changes() {
        let dir = std::env::temp_dir().join(format!(
            "meg-undo-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("file.txt");
        std::fs::write(&path, "before").unwrap();
        let checkpoint = FileCheckpoint::capture(&dir, &json!({"path":"file.txt"})).unwrap();
        std::fs::write(&path, "after").unwrap();
        checkpoint.commit(&dir).unwrap();
        std::fs::write(&path, "manual edit").unwrap();
        assert!(undo(&dir).unwrap_err().to_string().contains("refused"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "manual edit");
        std::fs::write(&path, "after").unwrap();
        undo(&dir).unwrap();
        assert_eq!(std::fs::read_to_string(path).unwrap(), "before");
    }
}
