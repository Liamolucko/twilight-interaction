use twilight_http::Client;

#[derive(Debug, Clone)]
pub struct Context {
    pub http: Client,
}
