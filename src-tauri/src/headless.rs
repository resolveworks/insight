use crate::core::Config;

/// Run the application in headless mode (no GUI)
pub fn run() {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("insight=info".parse().unwrap()),
        )
        .init();

    tracing::info!("Starting Insight in headless mode");

    let rt = tokio::runtime::Runtime::new().expect("Failed to create Tokio runtime");

    rt.block_on(async {
        let config = Config::load_or_default();
        config
            .ensure_dirs()
            .expect("Failed to create data directories");

        tracing::info!("Data directory: {:?}", config.data_dir);

        // TODO: Initialize iroh node
        // TODO: Start sync event handlers
        // TODO: Start search index maintenance

        tracing::info!("Headless server running. Press Ctrl+C to stop.");

        // Wait for shutdown signal
        tokio::signal::ctrl_c()
            .await
            .expect("Failed to listen for Ctrl+C");

        tracing::info!("Shutting down...");
    });
}
