use std::{collections::VecDeque, path::PathBuf, sync::Arc};

use crate::{
    decode::Pixels,
    folder::{FileVersion, Folder},
    render::Image,
};

// Prefetch stays at the normalized boundary; only admitted history holds GPU residents.
pub enum Ready {
    Pixels(Arc<Pixels>),
    Resident(Arc<Image>),
}

pub const BASE_BUDGET: usize = 512 * 1024 * 1024;

#[derive(Clone)]
pub struct Entry<T> {
    pub version: FileVersion,
    pub generation: u64,
    pub value: Arc<T>,
    pub bytes: usize,
    pub history: bool,
}

// Oldest history is first. Prefetch insertion does not refresh history recency.
pub struct Cache<T> {
    pub depth: usize,
    entries: VecDeque<Entry<T>>,
    largest: usize,
}

impl<T> Default for Cache<T> {
    fn default() -> Self {
        Self {
            depth: 1,
            entries: VecDeque::new(),
            largest: 0,
        }
    }
}

impl<T> Cache<T> {
    pub fn clear(&mut self) {
        self.entries.clear();
        self.largest = 0;
    }

    pub fn snapshot(&self) -> Vec<(FileVersion, u64, Arc<T>)> {
        self.entries
            .iter()
            .map(|entry| {
                (
                    entry.version.clone(),
                    entry.generation,
                    Arc::clone(&entry.value),
                )
            })
            .collect()
    }

    pub fn contains(&self, path: &PathBuf) -> bool {
        self.entries.iter().any(|entry| &entry.version.path == path)
    }

    pub fn admitted(&mut self, version: &FileVersion, bytes: usize) {
        self.largest = self.largest.max(bytes);
        self.entries
            .retain(|entry| entry.version.path != version.path);
    }

    pub fn insert(&mut self, entry: Entry<T>, displayed_bytes: usize) {
        self.entries
            .retain(|old| old.version.path != entry.version.path);
        self.entries.push_back(entry);
        self.trim(displayed_bytes);
    }

    pub fn reconcile(&mut self, folder: &Folder, displayed_bytes: usize) {
        let neighbors = neighbors(folder, self.depth);
        self.entries.retain(|entry| {
            folder.version(&entry.version.path) == Some(&entry.version)
                && (entry.history || neighbors.contains(&entry.version.path))
        });
        self.trim(displayed_bytes);
    }

    pub fn trim(&mut self, displayed_bytes: usize) {
        let budget = BASE_BUDGET.max(self.largest.saturating_mul(2));
        self.trim_to(displayed_bytes, budget);
    }

    fn trim_to(&mut self, displayed_bytes: usize, budget: usize) {
        while self.entries.iter().filter(|e| e.history).count() > self.depth {
            let index = self.entries.iter().position(|e| e.history).unwrap();
            self.entries.remove(index);
        }
        while self.entries.len() > self.depth * 2
            || self
                .entries
                .iter()
                .fold(displayed_bytes, |sum, e| sum.saturating_add(e.bytes))
                > budget
        {
            let index = self.entries.iter().position(|e| e.history).unwrap_or(0);
            if self.entries.remove(index).is_none() {
                break;
            }
        }
    }
}

pub fn neighbors(folder: &Folder, depth: usize) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(index) = folder.selected {
        for distance in 1..=depth {
            if let Some(path) = folder.files.get(index + distance) {
                paths.push(path.clone());
            }
            if let Some(path) = index
                .checked_sub(distance)
                .and_then(|i| folder.files.get(i))
            {
                paths.push(path.clone());
            }
        }
    }
    paths
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::folder::{Sort, tests::TempDir};
    use std::fs;

    fn fixture() -> (TempDir, Folder) {
        let dir = TempDir::new();
        for n in 0..=10 {
            fs::write(dir.0.join(format!("{n}.png")), [n]).unwrap();
        }
        let folder = Folder::open(dir.0.join("5.png")).unwrap();
        (dir, folder)
    }

    fn entry(folder: &Folder, n: usize, bytes: usize, history: bool) -> Entry<String> {
        let version = folder.version(&folder.files[n]).unwrap().clone();
        Entry {
            version,
            generation: 1,
            value: Arc::new(n.to_string()),
            bytes,
            history,
        }
    }

    #[test]
    fn depth_off_one_and_n_matrix() {
        let (_dir, folder) = fixture();
        for depth in 0..=5 {
            let mut cache = Cache {
                depth,
                ..Default::default()
            };
            assert_eq!(neighbors(&folder, depth).len(), depth * 2);
            for n in 0..11 {
                if n != 5 {
                    cache.insert(entry(&folder, n, 4, false), 4);
                }
            }
            assert_eq!(cache.entries.len(), depth * 2);
            for n in 0..11 {
                if n != 5 {
                    cache.insert(entry(&folder, n, 4, true), 4);
                }
            }
            assert_eq!(cache.entries.iter().filter(|e| e.history).count(), depth);
        }
        let mut boundary = folder.clone();
        boundary.selected = Some(0);
        assert_eq!(neighbors(&boundary, 5).len(), 5);
    }

    #[test]
    fn budget_evicts_oldest_history_then_prefetch_never_display() {
        let (_dir, folder) = fixture();
        let mut cache = Cache {
            depth: 5,
            ..Default::default()
        };
        cache.insert(entry(&folder, 1, 10, false), 10);
        cache.insert(entry(&folder, 2, 10, true), 10);
        cache.insert(entry(&folder, 3, 10, true), 10);
        cache.trim_to(10, 30);
        assert_eq!(
            cache
                .entries
                .iter()
                .map(|e| e.value.as_str())
                .collect::<Vec<_>>(),
            ["1", "3"]
        );
        cache.trim_to(10, 20);
        assert_eq!(cache.entries[0].value.as_str(), "1");
        cache.trim_to(10, 5);
        assert!(cache.entries.is_empty()); // Display is outside the eviction list.
    }

    #[test]
    fn admitted_large_image_derives_budget_and_promotes_history_recency() {
        let (_dir, folder) = fixture();
        let mut cache = Cache {
            depth: 2,
            ..Default::default()
        };
        cache.admitted(folder.version(&folder.files[5]).unwrap(), BASE_BUDGET);
        cache.insert(entry(&folder, 1, BASE_BUDGET, true), BASE_BUDGET);
        assert_eq!(cache.entries.len(), 1);
        cache.insert(entry(&folder, 2, 4, false), BASE_BUDGET);
        assert_eq!(cache.entries.len(), 1);
        assert_eq!(cache.entries[0].value.as_str(), "2");
        cache.clear();
        cache.insert(entry(&folder, 1, 4, true), 4);
        cache.insert(entry(&folder, 2, 4, true), 4);
        cache.admitted(folder.version(&folder.files[1]).unwrap(), 4);
        cache.insert(entry(&folder, 1, 4, true), 4);
        cache.insert(entry(&folder, 3, 4, true), 4);
        assert_eq!(
            cache
                .entries
                .iter()
                .map(|e| e.value.as_str())
                .collect::<Vec<_>>(),
            ["1", "3"]
        );
    }

    #[test]
    fn rescan_remove_change_and_sort_identity() {
        let (dir, mut folder) = fixture();
        let mut cache = Cache {
            depth: 5,
            ..Default::default()
        };
        cache.insert(entry(&folder, 1, 4, true), 4);
        cache.insert(entry(&folder, 2, 4, false), 4);
        cache.insert(entry(&folder, 3, 4, true), 4);
        folder.refresh(None, Sort::NameDescending).unwrap();
        cache.reconcile(&folder, 4);
        assert_eq!(cache.entries.len(), 3);
        fs::remove_file(dir.0.join("1.png")).unwrap();
        fs::write(dir.0.join("2.png"), b"changed").unwrap();
        folder.refresh(None, Sort::NameAscending).unwrap();
        cache.reconcile(&folder, 4);
        assert_eq!(cache.entries.len(), 1);
        assert_eq!(cache.entries[0].value.as_str(), "3");
    }

    #[test]
    fn depth_decrease_and_folder_clear_release_arcs_promptly() {
        let (_dir, folder) = fixture();
        let mut cache = Cache {
            depth: 5,
            ..Default::default()
        };
        let old = entry(&folder, 1, 4, true);
        let weak = Arc::downgrade(&old.value);
        cache.insert(old, 4);
        cache.insert(entry(&folder, 2, 4, true), 4);
        cache.depth = 1;
        cache.reconcile(&folder, 4);
        assert!(weak.upgrade().is_none());
        let weak = Arc::downgrade(&cache.entries[0].value);
        cache.clear();
        assert!(weak.upgrade().is_none());
        cache.insert(entry(&folder, 6, 4, false), 4);
        cache.depth = 0;
        cache.trim(4);
        assert!(cache.entries.is_empty());
    }

    #[test]
    fn envelope_eviction_releases_weak_resident() {
        let (_dir, folder) = fixture();
        let mut cache = Cache::default();
        let old = entry(&folder, 1, 4, true);
        let weak = Arc::downgrade(&old.value);
        cache.insert(old, 4);
        cache.trim_to(4, 4);
        assert!(weak.upgrade().is_none());
    }
}
