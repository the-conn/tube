#[tokio::main]
async fn main() -> Result<(), tube::TubeError> {
  let config = tube::tube_config::TubeConfig::load()?;
  let client = reqwest::Client::new();
  tube::run(config, client).await
}
