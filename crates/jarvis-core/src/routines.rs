use std::fs;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

pub const MAX_ROUTINES: usize = 50;
const MAX_NAME_CHARS: usize = 60;
const MAX_COMMAND_CHARS: usize = 500;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Routine {
    pub name: String,
    pub command: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RoutineStore {
    pub routines: Vec<Routine>,
}

impl RoutineStore {
    pub fn load(path: &Path) -> Self {
        match fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn save_atomic(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
        fs::write(&temporary, serde_json::to_vec_pretty(self)?)?;
        fs::rename(&temporary, path)
    }

    pub fn add(&mut self, name: &str, command: &str) -> Result<usize, String> {
        let name = name.trim();
        let command = command.trim();
        if name.is_empty() {
            return Err("Give the routine a name.".into());
        }
        if command.is_empty() {
            return Err("Add a command for the routine.".into());
        }
        if name.chars().count() > MAX_NAME_CHARS {
            return Err(format!("Keep the name under {MAX_NAME_CHARS} characters."));
        }
        if command.chars().count() > MAX_COMMAND_CHARS {
            return Err(format!(
                "Keep the command under {MAX_COMMAND_CHARS} characters."
            ));
        }
        if self.routines.len() >= MAX_ROUTINES {
            return Err(format!("You can save up to {MAX_ROUTINES} routines."));
        }
        if self
            .routines
            .iter()
            .any(|routine| routine.name.eq_ignore_ascii_case(name))
        {
            return Err("A routine with that name already exists.".into());
        }
        self.routines.push(Routine {
            name: name.to_string(),
            command: command.to_string(),
        });
        Ok(self.routines.len() - 1)
    }

    pub fn remove(&mut self, index: usize) -> Option<Routine> {
        (index < self.routines.len()).then(|| self.routines.remove(index))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_validates_and_rejects_duplicates() {
        let mut store = RoutineStore::default();
        assert_eq!(store.add("Focus", "open chrome and open vscode"), Ok(0));
        assert!(store.add("focus", "open notepad").is_err());
        assert!(store.add("", "open notepad").is_err());
        assert!(store.add("Empty", "   ").is_err());
        assert_eq!(store.routines[0].name, "Focus");
    }

    #[test]
    fn remove_and_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("routines.json");
        let mut store = RoutineStore::default();
        store.add("Focus", "open chrome").unwrap();
        store.add("Wind down", "close chrome").unwrap();
        store.save_atomic(&path).unwrap();
        assert_eq!(RoutineStore::load(&path), store);
        assert_eq!(store.remove(0).unwrap().name, "Focus");
        assert!(store.remove(9).is_none());
    }
}
