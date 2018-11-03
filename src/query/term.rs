use super::{make_field_value, CreateQuery, Result, Error};

use std::collections::HashMap;

use tantivy::query::{Query, TermQuery};
use tantivy::schema::{IndexRecordOption, Schema};

#[derive(Deserialize, Debug, PartialEq, Clone)]
pub struct ExactTerm {
    term: HashMap<String, String>,
}

impl CreateQuery for ExactTerm {
    fn create_query(self, schema: &Schema) -> Result<Box<Query>> {
        if let Some((k, v)) = self.term.into_iter().take(1).next() {
            let term = make_field_value(schema, &k, &v)?;
            Ok(Box::new(TermQuery::new(term, IndexRecordOption::Basic)))
        } else {
            Err(Error::QueryError("Query generation failed".into()))
        }
    }
}
