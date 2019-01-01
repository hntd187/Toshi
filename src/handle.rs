use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use log::debug;
use tantivy::collector::TopDocs;
use tantivy::query::{AllQuery, QueryParser};
use tantivy::schema::*;
use tantivy::{DocAddress, Document, Index, IndexWriter, Term};

use crate::handlers::index::{AddDocument, DeleteDoc, DocsAffected};
use crate::query::{CreateQuery, Query, Request};
use crate::results::{ScoredDoc, SearchResults};
use crate::settings::Settings;
use crate::Result;

pub enum IndexLocation {
    LOCAL,
    REMOTE,
}

pub trait IndexHandle {
    type SearchResponse;
    type DeleteResponse;
    type AddResponse;

    fn get_name(&self) -> String;
    fn index_location(&self) -> IndexLocation;
    fn search_index(&self, search: Request) -> Self::SearchResponse;
    fn add_document(&self, doc: AddDocument) -> Self::AddResponse;
    fn delete_term(&self, term: DeleteDoc) -> Self::DeleteResponse;
}

/// Index handle that operates on an Index local to the node, a remote index handle
/// will eventually call to wherever the local index is stored, so at some level the relevant
/// local handle will always get called through rpc
pub struct LocalIndex {
    index: Index,
    writer: Arc<Mutex<IndexWriter>>,
    current_opstamp: AtomicUsize,
    settings: Settings,
    name: String,
}

impl IndexHandle for LocalIndex {
    type SearchResponse = Result<SearchResults>;
    type DeleteResponse = Result<DocsAffected>;
    type AddResponse = Result<()>;

    fn get_name(&self) -> String {
        self.name.clone()
    }

    fn index_location(&self) -> IndexLocation {
        IndexLocation::LOCAL
    }

    fn search_index(&self, search: Request) -> Self::SearchResponse {
        self.index.load_searchers()?;
        let searcher = self.index.searcher();
        let schema = self.index.schema();
        let collector = TopDocs::with_limit(search.limit);
        let mut found_docs: Vec<(f32, DocAddress)> = Vec::new();
        if let Some(query) = search.query {
            match query {
                Query::Regex(regex) => {
                    let regex_query = regex.create_query(&schema)?;
                    found_docs = searcher.search(&*regex_query, &collector)?;
                }
                Query::Phrase(phrase) => {
                    let phrase_query = phrase.create_query(&schema)?;
                    found_docs = searcher.search(&*phrase_query, &collector)?;
                }
                Query::Fuzzy(fuzzy) => {
                    let fuzzy_query = fuzzy.create_query(&schema)?;
                    found_docs = searcher.search(&*fuzzy_query, &collector)?;
                }
                Query::Exact(term) => {
                    let exact_query = term.create_query(&schema)?;
                    found_docs = searcher.search(&*exact_query, &collector)?;
                }
                Query::Boolean { bool } => {
                    let bool_query = bool.create_query(&schema)?;
                    found_docs = searcher.search(&*bool_query, &collector)?;
                }
                Query::Range(range) => {
                    debug!("{:#?}", range);
                    let range_query = range.create_query(&schema)?;
                    debug!("{:?}", range_query);
                    found_docs = searcher.search(&*range_query, &collector)?;
                }
                Query::Raw { raw } => {
                    let fields: Vec<Field> = schema.fields().iter().filter_map(|e| schema.get_field(e.name())).collect();
                    let query_parser = QueryParser::for_index(&self.index, fields);
                    let query = query_parser.parse_query(&raw)?;
                    debug!("{:#?}", query);
                    found_docs = searcher.search(&*query, &collector)?;
                }
                Query::All => {
                    found_docs = searcher.search(&AllQuery, &collector)?;
                }
            }
        }

        let scored_docs: Vec<ScoredDoc> = found_docs
            .into_iter()
            .map(|(score, doc)| {
                let d = searcher.doc(doc).expect("Doc not found in segment");
                ScoredDoc::new(Some(score), schema.to_named_doc(&d))
            })
            .collect();

        Ok(SearchResults::new(scored_docs))
    }

    fn add_document(&self, add_doc: AddDocument) -> Self::AddResponse {
        let index_schema = self.index.schema();
        let writer_lock = self.get_writer();
        let mut index_writer = writer_lock.lock()?;
        let doc: Document = LocalIndex::parse_doc(&index_schema, &add_doc.document.to_string())?;
        index_writer.add_document(doc);
        if let Some(opts) = add_doc.options {
            if opts.commit {
                index_writer.commit().unwrap();
                self.set_opstamp(0);
            }
        } else {
            self.set_opstamp(self.get_opstamp() + 1);
        }
        Ok(())
    }

    fn delete_term(&self, term: DeleteDoc) -> Self::DeleteResponse {
        let index_schema = self.index.schema();
        let writer_lock = self.get_writer();
        let mut index_writer = writer_lock.lock()?;

        for (field, value) in term.terms {
            let f = index_schema.get_field(&field).unwrap();
            let term = Term::from_field_text(f, &value);
            index_writer.delete_term(term);
        }
        if let Some(opts) = term.options {
            if opts.commit {
                index_writer.commit().unwrap();
                self.set_opstamp(0);
            }
        }
        let docs_affected = self
            .index
            .load_metas()
            .map(|meta| meta.segments.iter().map(|seg| seg.num_deleted_docs()).sum())
            .unwrap_or(0);

        Ok(DocsAffected { docs_affected })
    }
}

impl LocalIndex {
    pub fn new(index: Index, settings: Settings, name: &str) -> Result<Self> {
        let i = index.writer(settings.writer_memory)?;
        i.set_merge_policy(settings.get_merge_policy());
        let current_opstamp = AtomicUsize::new(0);
        let writer = Arc::new(Mutex::new(i));
        Ok(Self {
            index,
            writer,
            current_opstamp,
            settings,
            name: name.into(),
        })
    }

    fn parse_doc(schema: &Schema, bytes: &str) -> Result<Document> {
        schema.parse_document(bytes).map_err(|e| e.into())
    }

    pub fn get_index(&self) -> &Index {
        &self.index
    }

    pub fn recreate_writer(self) -> Result<Self> {
        LocalIndex::new(self.index, self.settings.clone(), &self.name)
    }

    pub fn get_writer(&self) -> Arc<Mutex<IndexWriter>> {
        Arc::clone(&self.writer)
    }

    pub fn get_opstamp(&self) -> usize {
        self.current_opstamp.load(Ordering::Relaxed)
    }

    pub fn set_opstamp(&self, opstamp: usize) {
        self.current_opstamp.store(opstamp, Ordering::Relaxed)
    }
}
