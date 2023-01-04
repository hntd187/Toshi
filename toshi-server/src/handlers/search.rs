use std::sync::Arc;

use hyper::body::to_bytes;
use hyper::Response;
use hyper::{Body, StatusCode};
use log::info;

use toshi_types::*;

use crate::handlers::ResponseFuture;
use crate::utils::{empty_with_code, with_body};

pub async fn doc_search<C: Catalog>(catalog: Arc<C>, body: Body, index: &str) -> ResponseFuture {
    let b = to_bytes(body).await?;
    match serde_json::from_slice::<Search>(&b) {
        Ok(req) => {
            let req = if req.query.is_none() { Search::all_limit(req.limit) } else { req };
            if catalog.exists(index) {
                info!("Query: {:?}", req);
                let index = catalog.get_index(index).unwrap(); // If this unwrap fails, this is a bug.
                match index.search_index(req).await {
                    Ok(results) => Ok(with_body(results)),
                    Err(e) => Ok(Response::from(e)),
                }
            } else {
                Ok(empty_with_code(StatusCode::NOT_FOUND))
            }
        }
        Err(err) => Ok(Response::from(Error::QueryError(format!("Bad JSON Query: {}", err)))),
    }
}

pub async fn all_docs<C: Catalog>(catalog: Arc<C>, index: &str) -> ResponseFuture {
    let body = Body::from(serde_json::to_vec(&Search::all_docs()).unwrap());
    doc_search(catalog, body, index).await
}

#[cfg(test)]
pub mod tests {
    use std::sync::Arc;

    use hyper::Body;
    use pretty_assertions::assert_eq;

    use toshi_types::{ErrorResponse, ExactTerm, FuzzyQuery, FuzzyTerm, KeyValue, PhraseQuery, Query, Search, TermPair};

    use crate::commit::tests::*;
    use crate::handlers::{doc_search, ResponseFuture};
    use crate::index::create_test_catalog;
    use crate::SearchResults;

    type ReturnUnit = Result<(), Box<dyn std::error::Error>>;

    pub async fn run_query(req: Search, index: &str) -> ResponseFuture {
        let cat = create_test_catalog(index);
        doc_search(Arc::clone(&cat), Body::from(serde_json::to_vec(&req).unwrap()), index).await
    }

    #[tokio::test]
    async fn test_term_query() -> Result<(), Box<dyn std::error::Error>> {
        let term = KeyValue::new("test_text".into(), "document".into());
        let term_query = Query::Exact(ExactTerm::new(term));
        let search = Search::new(Some(term_query), None, 10, None);
        let q = run_query(search, "test_index").await?;
        let body: SearchResults = wait_json(q).await;
        assert_eq!(body.hits, 3);
        Ok(())
    }

    #[tokio::test]
    async fn test_phrase_query() -> Result<(), Box<dyn std::error::Error>> {
        let terms = TermPair::new(vec!["test".into(), "document".into()], None);
        let phrase = KeyValue::new("test_text".into(), terms);
        let term_query = Query::Phrase(PhraseQuery::new(phrase));
        let search = Search::new(Some(term_query), None, 10, None);
        let q = run_query(search, "test_index").await?;
        let body: SearchResults = wait_json(q).await;
        assert_eq!(body.hits, 3);
        Ok(())
    }

    #[tokio::test]
    async fn test_bad_raw_query_syntax() -> ReturnUnit {
        let cat = create_test_catalog("test_index");
        let body = r#"{ "query" : { "raw": "asd*(@sq__" } }"#;
        let err = doc_search(Arc::clone(&cat), Body::from(body), "test_index").await?;
        let body: ErrorResponse = wait_json::<ErrorResponse>(err).await;
        assert_eq!(body.message, "Error in Index: \'Syntax Error: asd*(@sq__\'");
        Ok(())
    }

    #[tokio::test]
    async fn test_unindexed_field() -> ReturnUnit {
        let cat = create_test_catalog("test_index");
        let body = r#"{ "query" : { "raw": "test_unindex:yes" } }"#;
        let r = doc_search(Arc::clone(&cat), Body::from(body), "test_index").await?;
        let b = read_body(r).await?;
        let expected = r#"{"message":"Error in Index: 'The field 'test_unindex' is not declared as indexed'"}"#;
        assert_eq!(b, expected);
        Ok(())
    }

    #[tokio::test]
    async fn test_bad_term_field_syntax() -> ReturnUnit {
        let cat = create_test_catalog("test_index");
        let body = r#"{ "query" : { "term": { "asdf": "Document" } } }"#;
        let q = doc_search(Arc::clone(&cat), Body::from(body), "test_index").await?;
        let b: ErrorResponse = wait_json(q).await;
        assert_eq!(b.message, "Error in query execution: 'Unknown field: asdf'");
        Ok(())
    }

    #[tokio::test]
    async fn test_facets() -> ReturnUnit {
        let body = r#"{ "query" : { "term": { "test_text": "document" } }, "facets": { "test_facet": ["/cat"] } }"#;
        let req: Search = serde_json::from_str(body)?;
        let q = run_query(req, "test_index").await?;
        let b: SearchResults = wait_json(q).await;
        assert_eq!(b.get_facets()[0].value, 1);
        assert_eq!(b.get_facets()[1].value, 1);
        assert_eq!(b.get_facets()[0].field, "/cat/cat2");
        Ok(())
    }

    // This code is just...the worst thing ever.
    #[tokio::test]
    async fn test_raw_query() -> ReturnUnit {
        let b = r#"test_text:"Duckiment""#;
        let req = Search::new(Some(Query::Raw { raw: b.into() }), None, 10, None);
        let q = run_query(req, "test_index").await?;
        let body: SearchResults = wait_json(q).await;
        assert_eq!(body.hits as usize, body.get_docs().len());
        let b2 = body;
        let map = b2.get_docs()[0].clone().doc.0;
        let text = String::from(map.remove("test_text").unwrap().1.as_str().unwrap());
        assert_eq!(text, "Test Duckiment 3");
        Ok(())
    }

    #[tokio::test]
    async fn test_fuzzy_term_query() -> ReturnUnit {
        let fuzzy = KeyValue::new("test_text".into(), FuzzyTerm::new("document".into(), 0, false));
        let term_query = Query::Fuzzy(FuzzyQuery::new(fuzzy));
        let search = Search::new(Some(term_query), None, 10, None);
        let q = run_query(search, "test_index").await?;
        let body: SearchResults = wait_json(q).await;

        assert_eq!(body.hits as usize, body.get_docs().len());
        assert_eq!(body.hits, 3);
        assert_eq!(body.get_docs().len(), 3);
        Ok(())
    }

    #[tokio::test]
    async fn test_inclusive_range_query() -> ReturnUnit {
        let body = r#"{ "query" : { "range" : { "test_i64" : { "gte" : 2012, "lte" : 2015 } } } }"#;
        let req: Search = serde_json::from_str(body)?;
        let q = run_query(req, "test_index").await?;
        let body: SearchResults = wait_json(q).await;
        assert_eq!(body.hits as usize, body.get_docs().len());
        assert!(cmp_float(body.get_docs()[0].score.unwrap(), 1.0));
        Ok(())
    }

    #[tokio::test]
    async fn test_exclusive_range_query() -> ReturnUnit {
        let body = r#"{ "query" : { "range" : { "test_i64" : { "gt" : 2012, "lt" : 2015 } } } }"#;
        let req: Search = serde_json::from_str(body)?;
        let q = run_query(req, "test_index").await?;
        let body: SearchResults = wait_json(q).await;
        assert_eq!(body.hits as usize, body.get_docs().len());
        assert!(cmp_float(body.get_docs()[0].score.unwrap(), 1.0));
        Ok(())
    }

    #[tokio::test]
    async fn test_regex_query() -> ReturnUnit {
        let body = r#"{ "query" : { "regex" : { "test_text" : "d[ou]{1}c[k]?ument" } } }"#;
        let req: Search = serde_json::from_str(body)?;
        let q = run_query(req, "test_index").await?;
        let body: SearchResults = wait_json(q).await;
        assert_eq!(body.hits, 4);
        Ok(())
    }

    #[tokio::test]
    async fn test_bool_query() -> ReturnUnit {
        let test_json = r#"{"query": { "bool": {
                "must": [ { "term": { "test_text": "document" } } ],
                "must_not": [ {"range": {"test_i64": { "gt": 2017 } } } ] } } }"#;

        let query = serde_json::from_str::<Search>(test_json)?;
        let q = run_query(query, "test_index").await?;
        let body: SearchResults = wait_json(q).await;
        assert_eq!(body.hits, 2);
        Ok(())
    }
}
