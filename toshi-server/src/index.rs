use std::clone::Clone;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use dashmap::DashMap;
use tantivy::schema::Schema;
use tantivy::Index;

use toshi_types::{Catalog, Error};

use crate::handle::LocalIndex;
use crate::settings::Settings;
use crate::Result;

pub type SharedCatalog = Arc<IndexCatalog>;

pub struct IndexCatalog {
    settings: Settings,
    base_path: PathBuf,
    local_handles: DashMap<String, LocalIndex>,
}

impl IndexCatalog {
    pub fn get_settings(&self) -> &Settings {
        &self.settings
    }
}

#[async_trait::async_trait]
impl Catalog for IndexCatalog {
    type Handle = LocalIndex;

    fn base_path(&self) -> String {
        format!("{}", self.base_path.display())
    }

    fn get_collection(&self) -> &DashMap<String, Self::Handle> {
        &self.local_handles
    }

    fn add_index(&self, name: &str, schema: Schema) -> Result<()> {
        let handle = LocalIndex::new(
            self.base_path.clone(),
            name,
            schema,
            self.settings.writer_memory,
            self.settings.get_merge_policy(),
        )?;
        self.local_handles.insert(name.to_string(), handle);
        Ok(())
    }

    async fn list_indexes(&self) -> Vec<String> {
        let mut local_keys = self.local_handles.iter().map(|e| e.key().to_owned()).collect::<Vec<String>>();
        local_keys.sort();
        local_keys.dedup();
        local_keys
    }

    fn get_index(&self, name: &str) -> Result<Self::Handle> {
        self.local_handles
            .get(name)
            .map(|r| r.value().to_owned())
            .ok_or_else(|| Error::UnknownIndex(name.into()))
    }

    fn exists(&self, index: &str) -> bool {
        self.get_collection().contains_key(index)
    }

    fn raft_id(&self) -> u64 {
        self.settings.experimental_features.id
    }
}

impl IndexCatalog {
    pub fn new(settings: Settings) -> Result<Self> {
        let local_idxs = DashMap::new();
        let path = PathBuf::from(&settings.path);
        let mut index_cat = IndexCatalog {
            settings,
            base_path: path,
            local_handles: local_idxs,
        };
        index_cat.refresh_catalog()?;

        Ok(index_cat)
    }

    pub fn load_index(path: &str) -> Result<Index> {
        let p = PathBuf::from(path);
        if p.exists() {
            Index::open_in_dir(&p).map_err(|_| Error::UnknownIndex(p.display().to_string()))
        } else {
            Err(Error::UnknownIndex(path.to_string()))
        }
    }

    pub fn get_mut_collection(&mut self) -> &mut DashMap<String, LocalIndex> {
        &mut self.local_handles
    }

    #[allow(dead_code)]
    pub(crate) fn add_test_index(&mut self, name: String, index: Index) {
        let local = LocalIndex::with_existing(name.clone(), index).unwrap();
        self.local_handles.insert(name, local);
    }

    pub fn refresh_catalog(&mut self) -> Result<()> {
        self.local_handles.clear();

        for dir in fs::read_dir(self.base_path.clone())? {
            let entry = dir?.path();
            if let Some(entry_str) = entry.to_str() {
                if entry.exists() {
                    if !entry_str.ends_with(".node_id") {
                        let pth: String = entry_str.rsplit('/').take(1).collect();
                        let idx = IndexCatalog::load_index(entry_str)?;
                        self.add_index(&pth, idx.schema())?;
                    }
                } else {
                    return Err(Error::UnknownIndex(format!("Path {}", entry.display())));
                }
            } else {
                return Err(Error::UnknownIndex(format!("Path {} is not a valid unicode path", entry.display())));
            }
        }
        Ok(())
    }

    pub async fn clear(&self) {
        self.local_handles.clear();
    }

    #[doc(hidden)]
    #[allow(dead_code)]
    pub fn with_index(name: String, index: Index) -> Result<Self> {
        let map = DashMap::new();
        let settings = Settings {
            json_parsing_threads: 1,
            ..Default::default()
        };
        let new_index = LocalIndex::with_existing(name.clone(), index)
            .unwrap_or_else(|e| panic!("Unable to open index: {} because it's locked: {:?}", name, e));

        map.insert(name, new_index);

        Ok(IndexCatalog {
            settings,
            base_path: PathBuf::new(),
            local_handles: map,
        })
    }
}

#[cfg(test)]
pub fn create_test_catalog(name: &str) -> SharedCatalog {
    let idx = toshi_test::create_test_index();
    let catalog = IndexCatalog::with_index(name.into(), idx).unwrap();
    Arc::new(catalog)
}
