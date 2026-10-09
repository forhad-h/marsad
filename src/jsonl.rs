use serde_json::Value;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;

/// Incrementally reads new complete lines from an append-only JSONL file.
pub struct JsonlTail {
    pub path: PathBuf,
    offset: u64,
    partial: Vec<u8>,
}

impl JsonlTail {
    pub fn new(path: PathBuf) -> Self {
        Self { path, offset: 0, partial: Vec::new() }
    }

    /// Returns values parsed from lines appended since the last call.
    /// If the file shrank (rotated or rewritten), it is re-read from the start.
    pub fn read_new(&mut self) -> Vec<Value> {
        let Ok(mut file) = File::open(&self.path) else { return Vec::new() };
        let Ok(len) = file.metadata().map(|m| m.len()) else { return Vec::new() };
        if len < self.offset {
            self.offset = 0;
            self.partial.clear();
        }
        if len == self.offset || file.seek(SeekFrom::Start(self.offset)).is_err() {
            return Vec::new();
        }
        let mut chunk = Vec::with_capacity((len - self.offset) as usize);
        if file.read_to_end(&mut chunk).is_err() {
            return Vec::new();
        }
        self.offset += chunk.len() as u64;
        self.partial.extend_from_slice(&chunk);

        let Some(last_nl) = self.partial.iter().rposition(|&b| b == b'\n') else { return Vec::new() };
        let rest = self.partial.split_off(last_nl + 1);
        let complete = std::mem::replace(&mut self.partial, rest);
        complete
            .split(|&b| b == b'\n')
            .filter(|l| !l.is_empty())
            .filter_map(|l| serde_json::from_slice(l).ok())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn reads_only_complete_lines_incrementally() {
        let dir = std::env::temp_dir().join(format!("marsad-jsonl-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.jsonl");
        let mut f = File::create(&path).unwrap();
        write!(f, "{{\"a\":1}}\n{{\"a\":").unwrap();
        f.flush().unwrap();
        let mut tail = JsonlTail::new(path.clone());
        assert_eq!(tail.read_new().len(), 1);
        write!(f, "2}}\n").unwrap();
        f.flush().unwrap();
        let v = tail.read_new();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0]["a"], 2);
        assert!(tail.read_new().is_empty());
        std::fs::remove_dir_all(dir).ok();
    }
}
