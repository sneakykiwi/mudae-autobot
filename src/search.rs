use tokio::sync::{mpsc, oneshot};

#[derive(Debug, Clone, PartialEq)]
pub struct SearchResult {
    pub name: String,
    pub series: String,
    pub image_url: Option<String>,
    pub kakera_value: Option<u32>,
    pub exists: bool,
}

#[derive(Debug)]
pub struct SearchRequest {
    pub query: String,
    pub channel_id: u64,
    pub response_tx: oneshot::Sender<Option<SearchResult>>,
}

pub type SearchRequestSender = mpsc::Sender<SearchRequest>;
pub type SearchRequestReceiver = mpsc::Receiver<SearchRequest>;

pub fn create_search_channel() -> (SearchRequestSender, SearchRequestReceiver) {
    mpsc::channel(16)
}
