use std::collections::HashMap;

use crate::types::{
    DataSourceError, DataSourceProvider, DataSourceQuery, DataSourceResponse, DataSourceResult,
};

pub struct DataSourceRegistry {
    providers: HashMap<String, Box<dyn DataSourceProvider>>,
}

impl Default for DataSourceRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl DataSourceRegistry {
    pub fn new() -> Self {
        Self {
            providers: HashMap::new(),
        }
    }

    pub fn register(&mut self, provider: Box<dyn DataSourceProvider>) {
        self.providers.insert(provider.id().to_string(), provider);
    }

    pub fn get(&self, id: &str) -> Option<&dyn DataSourceProvider> {
        self.providers.get(id).map(|b| b.as_ref())
    }

    pub fn list_ids(&self) -> Vec<String> {
        let mut ids: Vec<_> = self.providers.keys().cloned().collect();
        ids.sort();
        ids
    }

    pub async fn query(
        &self,
        id: &str,
        q: DataSourceQuery,
    ) -> DataSourceResult<DataSourceResponse> {
        let provider = self
            .providers
            .get(id)
            .ok_or_else(|| DataSourceError::Other(format!("unknown provider: {id}")))?;
        provider.query(q).await
    }
}
