use reqwest::{Client, Response};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::{Semaphore, SemaphorePermit};
use tokio::time::MissedTickBehavior;

/// Performance metrics for monitoring request behavior
#[derive(Debug, Default)]
pub struct RequestMetrics {
    /// Total number of requests made
    pub total_requests: AtomicU64,
    /// Number of successful requests
    pub successful_requests: AtomicU64,
    /// Number of failed requests
    pub failed_requests: AtomicU64,
    /// Number of currently active requests
    pub active_requests: AtomicUsize,
    /// Peak concurrent requests observed
    pub peak_concurrent: AtomicUsize,
}

impl RequestMetrics {
    /// Get a formatted summary of metrics
    pub fn summary(&self) -> String {
        let total = self.total_requests.load(Ordering::Relaxed);
        let success = self.successful_requests.load(Ordering::Relaxed);
        let failed = self.failed_requests.load(Ordering::Relaxed);
        let active = self.active_requests.load(Ordering::Relaxed);
        let peak = self.peak_concurrent.load(Ordering::Relaxed);

        format!(
            "Requests - Total: {}, Success: {}, Failed: {}, Active: {}, Peak: {}",
            total, success, failed, active, peak
        )
    }
}

/// A high-performance HTTP/2 connection pool manager with advanced features:
/// - Pre-warmed connections for minimal latency
/// - HTTP/2 multiplexing with optimized window sizes
/// - Adaptive keep-alive strategies
/// - Comprehensive metrics tracking
/// - Configurable concurrency limits
///
/// # Architecture
/// Uses a single shared HTTP client with HTTP/2 multiplexing to minimize
/// connection overhead while maintaining high concurrency. The semaphore-based
/// concurrency control ensures predictable resource usage.
///
/// # Example
/// ```rust,no_run
/// # async fn example() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
/// use sayna::utils::req_manager::ReqManager;
///
/// // Create manager with 10 concurrent request limit
/// let manager = ReqManager::new(10).await?;
///
/// // Warm up connections for optimal performance
/// manager.aggressive_warmup("https://api.example.com/health").await?;
///
/// // Acquire a client and make a request
/// let guard = manager.acquire().await?;
/// let response = guard.client()
///     .get("https://api.example.com/data")
///     .send()
///     .await?;
///
/// // Check metrics
/// println!("Metrics: {}", manager.metrics().summary());
///
/// // Client is automatically returned to pool when guard is dropped
/// # Ok(())
/// # }
/// ```
pub struct ReqManager {
    /// Maximum number of concurrent requests allowed
    max_concurrent_requests: usize,

    /// A single, long-lived HTTP client with connection pooling and HTTP/2 multiplexing
    client: Arc<Client>,

    /// Semaphore to control concurrent access to clients
    semaphore: Arc<Semaphore>,

    /// Performance metrics
    metrics: Arc<RequestMetrics>,
}

/// A guard that holds a client from the pool and returns it when dropped.
/// Automatically tracks metrics and handles cleanup.
pub struct ClientGuard<'a> {
    manager: &'a ReqManager,
    client: Arc<Client>,
    _permit: SemaphorePermit<'a>,
    _request_start: Instant,
}

impl<'a> ClientGuard<'a> {
    /// Get the HTTP client for making requests
    pub fn client(&self) -> &Client {
        &self.client
    }

    /// Make a GET request with automatic metrics tracking
    pub async fn get(&self, url: &str) -> Result<Response, reqwest::Error> {
        self.manager
            .metrics
            .total_requests
            .fetch_add(1, Ordering::Relaxed);
        let result = self.client.get(url).send().await;
        self.update_metrics(&result);
        result
    }

    /// Make a POST request with automatic metrics tracking
    pub async fn post(&self, url: &str) -> Result<Response, reqwest::Error> {
        self.manager
            .metrics
            .total_requests
            .fetch_add(1, Ordering::Relaxed);
        let result = self.client.post(url).send().await;
        self.update_metrics(&result);
        result
    }

    /// Update metrics based on request result
    fn update_metrics(&self, result: &Result<Response, reqwest::Error>) {
        match result {
            Ok(_) => {
                self.manager
                    .metrics
                    .successful_requests
                    .fetch_add(1, Ordering::Relaxed);
            }
            Err(_) => {
                self.manager
                    .metrics
                    .failed_requests
                    .fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

impl<'a> Drop for ClientGuard<'a> {
    fn drop(&mut self) {
        self.manager
            .metrics
            .active_requests
            .fetch_sub(1, Ordering::Relaxed);
    }
}

/// Configuration for the HTTP request manager
#[derive(Debug, Clone)]
pub struct ReqManagerConfig {
    /// Maximum number of concurrent requests
    pub max_concurrent_requests: usize,
    /// Initial stream window size for HTTP/2 (bytes)
    pub http2_stream_window_size: u32,
    /// Initial connection window size for HTTP/2 (bytes)
    pub http2_connection_window_size: u32,
    /// Keep-alive interval for HTTP/2 connections
    pub http2_keep_alive_interval: Duration,
    /// Keep-alive timeout for HTTP/2 connections
    pub http2_keep_alive_timeout: Duration,
    /// Maximum idle connections per host
    pub pool_max_idle_per_host: usize,
    /// TCP keep-alive duration
    pub tcp_keepalive: Duration,
    /// Connection timeout
    pub connect_timeout: Duration,
    /// Request timeout
    pub request_timeout: Duration,
}

impl Default for ReqManagerConfig {
    fn default() -> Self {
        Self {
            max_concurrent_requests: 10,
            http2_stream_window_size: 8_388_608,      // 8MB
            http2_connection_window_size: 16_777_216, // 16MB
            http2_keep_alive_interval: Duration::from_secs(3),
            http2_keep_alive_timeout: Duration::from_secs(10),
            pool_max_idle_per_host: 512,
            tcp_keepalive: Duration::from_secs(5),
            connect_timeout: Duration::from_millis(500),
            request_timeout: Duration::from_secs(30),
        }
    }
}

impl ReqManagerConfig {
    /// Create a low-latency configuration optimized for fast response times
    pub fn low_latency() -> Self {
        Self {
            max_concurrent_requests: 20,
            http2_stream_window_size: 4_194_304,     // 4MB
            http2_connection_window_size: 8_388_608, // 8MB
            http2_keep_alive_interval: Duration::from_secs(2),
            http2_keep_alive_timeout: Duration::from_secs(5),
            pool_max_idle_per_host: 1024,
            tcp_keepalive: Duration::from_secs(3),
            connect_timeout: Duration::from_millis(250),
            request_timeout: Duration::from_secs(15),
        }
    }

    /// Create a high-throughput configuration optimized for bulk operations
    pub fn high_throughput() -> Self {
        Self {
            max_concurrent_requests: 50,
            http2_stream_window_size: 16_777_216,     // 16MB
            http2_connection_window_size: 33_554_432, // 32MB
            http2_keep_alive_interval: Duration::from_secs(5),
            http2_keep_alive_timeout: Duration::from_secs(15),
            pool_max_idle_per_host: 256,
            tcp_keepalive: Duration::from_secs(10),
            connect_timeout: Duration::from_millis(1000),
            request_timeout: Duration::from_secs(60),
        }
    }
}

impl ReqManager {
    /// Create a new request manager with the specified maximum concurrent requests
    ///
    /// # Arguments
    /// * `max_concurrent_requests` - Maximum number of concurrent requests allowed (1-1000)
    ///
    /// # Returns
    /// A new `ReqManager` instance with default configuration
    pub async fn new(
        max_concurrent_requests: usize,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let config = ReqManagerConfig {
            max_concurrent_requests,
            ..Default::default()
        };
        Self::with_config(config).await
    }

    /// Create a new request manager with custom configuration
    ///
    /// # Arguments
    /// * `config` - Custom configuration for the manager
    ///
    /// # Returns
    /// A new `ReqManager` instance with the specified configuration
    ///
    /// # Example
    /// ```no_run
    /// # async fn example() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    /// use sayna::utils::req_manager::{ReqManager, ReqManagerConfig};
    ///
    /// // Use low-latency configuration
    /// let config = ReqManagerConfig::low_latency();
    /// let manager = ReqManager::with_config(config).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn with_config(
        config: ReqManagerConfig,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        if config.max_concurrent_requests == 0 {
            return Err("max_concurrent_requests must be greater than 0".into());
        }
        if config.max_concurrent_requests > 1000 {
            return Err("max_concurrent_requests must not exceed 1000".into());
        }

        let client = Arc::new(Self::create_optimized_client(&config)?);

        Ok(Self {
            max_concurrent_requests: config.max_concurrent_requests,
            client,
            semaphore: Arc::new(Semaphore::new(config.max_concurrent_requests)),
            metrics: Arc::new(RequestMetrics::default()),
        })
    }

    /// Create an optimized HTTP/2 client with advanced connection pooling
    fn create_optimized_client(config: &ReqManagerConfig) -> Result<Client, reqwest::Error> {
        Client::builder()
            .http2_initial_stream_window_size(config.http2_stream_window_size)
            .http2_initial_connection_window_size(config.http2_connection_window_size)
            .http2_keep_alive_interval(Some(config.http2_keep_alive_interval))
            .http2_keep_alive_timeout(config.http2_keep_alive_timeout)
            .http2_keep_alive_while_idle(true)
            .http2_adaptive_window(true)
            .pool_idle_timeout(None)
            .pool_max_idle_per_host(config.pool_max_idle_per_host)
            .tcp_keepalive(config.tcp_keepalive)
            .tcp_nodelay(true)
            .connect_timeout(config.connect_timeout)
            .timeout(config.request_timeout)
            .user_agent("sayna-req-manager/2.0")
            .build()
    }

    /// Acquire a client from the pool with automatic metrics tracking
    ///
    /// This method will wait if all clients are currently in use.
    /// The returned `ClientGuard` will automatically return the client
    /// to the pool when dropped.
    ///
    /// # Returns
    /// A `ClientGuard` that provides access to an HTTP client
    ///
    /// # Performance
    /// - Zero allocation after initial setup
    /// - Sub-microsecond acquisition time when permits available
    /// - Automatic connection reuse via HTTP/2 multiplexing
    pub async fn acquire(
        &self,
    ) -> Result<ClientGuard<'_>, Box<dyn std::error::Error + Send + Sync>> {
        // Acquire semaphore permit to ensure we don't exceed max concurrent requests
        let permit = self.semaphore.acquire().await?;

        // Update metrics
        let active = self.metrics.active_requests.fetch_add(1, Ordering::Relaxed) + 1;
        self.metrics
            .peak_concurrent
            .fetch_max(active, Ordering::Relaxed);

        let client = Arc::clone(&self.client);

        Ok(ClientGuard {
            manager: self,
            client,
            _permit: permit,
            _request_start: Instant::now(),
        })
    }

    /// Warm up connections by sending lightweight requests
    ///
    /// Establishes HTTP/2 connections and negotiates protocol parameters
    /// to eliminate first-request latency.
    ///
    /// # Arguments
    /// * `url` - The URL to warm up connections with
    /// * `warmup_type` - Type of warmup: "HEAD", "OPTIONS", or "GET"
    ///
    /// # Performance Impact
    /// Can reduce first-request latency from 100-500ms to 10-50ms
    pub async fn warmup(
        &self,
        url: &str,
        warmup_type: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        use futures::future::join_all;

        // Use multiple concurrent lightweight requests to prime TLS, ALPN, and pools.
        let parallel_streams = self.max_concurrent_requests.clamp(1, 16); // Increased max for better warmup

        let warmup_futures: Vec<_> = (0..parallel_streams)
            .map(|i| {
                let client = Arc::clone(&self.client);
                let url = url.to_string();
                let warmup_type = warmup_type.to_string();

                async move {
                    let start = Instant::now();

                    let result = match warmup_type.as_str() {
                        "HEAD" => {
                            client
                                .head(&url)
                                .timeout(Duration::from_millis(500))
                                .send()
                                .await
                        }
                        "OPTIONS" => {
                            client
                                .request(reqwest::Method::OPTIONS, &url)
                                .timeout(Duration::from_millis(500))
                                .send()
                                .await
                        }
                        "GET" => {
                            client
                                .get(&url)
                                .timeout(Duration::from_millis(500))
                                .send()
                                .await
                        }
                        _ => {
                            client
                                .request(reqwest::Method::OPTIONS, &url)
                                .timeout(Duration::from_millis(500))
                                .send()
                                .await
                        }
                    };

                    match result {
                        Ok(resp) => {
                            // Consume response to properly reuse connection
                            let _ = resp.bytes().await;
                            let elapsed = start.elapsed();
                            if i < 5 {
                                // Only log first few to avoid spam
                                println!("  Warmup stream {i} completed in {elapsed:?}");
                            }
                            Ok(elapsed)
                        }
                        Err(e) if e.is_timeout() => {
                            // Timeout during warmup is acceptable
                            Ok(Duration::from_millis(500))
                        }
                        Err(e) => {
                            if i < 5 {
                                eprintln!("  Warmup stream {i} failed: {e}");
                            }
                            Err(e)
                        }
                    }
                }
            })
            .collect();

        let results = join_all(warmup_futures).await;

        // Calculate average latency for diagnostics
        let successful_times: Vec<Duration> = results
            .iter()
            .filter_map(|r| r.as_ref().ok())
            .cloned()
            .collect();

        if !successful_times.is_empty() {
            let avg_ms = successful_times.iter().map(|d| d.as_millis()).sum::<u128>()
                / successful_times.len() as u128;
            println!(
                "Warmup complete: {} streams, avg latency: {}ms",
                successful_times.len(),
                avg_ms
            );
        }

        Ok(())
    }

    /// Aggressive warmup using intelligent phased approach
    ///
    /// Performs a two-phase warmup:
    /// 1. Initial probe to measure latency
    /// 2. Aggressive parallel warmup if latency is acceptable
    ///
    /// # Arguments
    /// * `url` - The URL to warm up connections with
    ///
    /// # Performance
    /// Can establish 100+ concurrent HTTP/2 streams in under 100ms
    pub async fn aggressive_warmup(
        &self,
        url: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        use futures::future::join_all;

        println!("Starting intelligent aggressive warmup");

        // Phase 1: Probe with smaller number of requests
        let probe_count = (self.max_concurrent_requests / 2).max(1);
        let probe_futures: Vec<_> = (0..probe_count)
            .map(|_| {
                let client = Arc::clone(&self.client);
                let url = url.to_string();
                async move {
                    let start = Instant::now();
                    let result = client
                        .head(&url)
                        .timeout(Duration::from_millis(300))
                        .send()
                        .await;
                    match result {
                        Ok(resp) => {
                            let _ = resp.bytes().await;
                            Ok(start.elapsed())
                        }
                        Err(_) => Err(()),
                    }
                }
            })
            .collect();

        let probe_results = join_all(probe_futures).await;
        let avg_latency_ms = probe_results
            .iter()
            .filter_map(|r| r.as_ref().ok())
            .map(|d| d.as_millis())
            .sum::<u128>()
            / probe_count as u128;

        println!("  Phase 1 probe: avg latency {}ms", avg_latency_ms);

        // Phase 2: Full warmup if latency is good
        if avg_latency_ms < 200 {
            let warmup_count = (self.max_concurrent_requests * 3).min(100); // Cap at 100

            let warmup_futures: Vec<_> = (0..warmup_count)
                .map(|_i| {
                    let client = Arc::clone(&self.client);
                    let url = url.to_string();

                    async move {
                        // Use very short timeout for aggressive warmup
                        let result = client
                            .head(&url)
                            .timeout(Duration::from_millis(100))
                            .send()
                            .await;

                        match result {
                            Ok(resp) => {
                                let _ = resp.bytes().await;
                                Ok(())
                            }
                            Err(e) if e.is_timeout() => Ok(()), // Timeout is fine for warmup
                            Err(_) => Err(()),
                        }
                    }
                })
                .collect();

            let results = join_all(warmup_futures).await;
            let successful = results.iter().filter(|r| r.is_ok()).count();

            println!(
                "  Phase 2 aggressive: {}/{} successful",
                successful, warmup_count
            );
        } else {
            println!("  Skipping phase 2 due to high latency");
        }

        Ok(())
    }

    /// Start an adaptive keep-alive task that adjusts to server behavior
    ///
    /// Maintains HTTP/2 connections with intelligent interval adjustment:
    /// - Starts with 3s interval
    /// - Increases to 10s if stable
    /// - Decreases to 2s if failures detected
    /// - Stops after 10 consecutive failures
    ///
    /// # Arguments
    /// * `url` - The URL to send keep-alive requests to (should be a lightweight endpoint)
    ///
    /// # Returns
    /// A JoinHandle for the spawned task that can be used to cancel the keep-alive loop
    pub fn start_keepalive_task(&self, url: String) -> tokio::task::JoinHandle<()> {
        let client = Arc::clone(&self.client);
        let _metrics = Arc::clone(&self.metrics);

        println!("Starting adaptive keep-alive task for URL: {}", url);

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(3));
            interval.set_missed_tick_behavior(MissedTickBehavior::Skip);

            let mut consecutive_failures = 0;
            let mut current_interval_secs = 3;

            loop {
                interval.tick().await;

                // Send lightweight HEAD request to keep connection alive
                let result = client
                    .head(&url)
                    .header("X-Keep-Alive", "adaptive")
                    .timeout(Duration::from_millis(50)) // Very short timeout
                    .send()
                    .await;

                match result {
                    Ok(_) => {
                        consecutive_failures = 0;

                        // Gradually increase interval if stable (up to 10s)
                        if current_interval_secs < 10 {
                            current_interval_secs += 1;
                            interval =
                                tokio::time::interval(Duration::from_secs(current_interval_secs));
                            interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
                        }
                    }
                    Err(e) if e.is_timeout() => {
                        // Timeout is acceptable for keep-alive
                        consecutive_failures = 0;
                    }
                    Err(e) => {
                        consecutive_failures += 1;

                        // Decrease interval if failing (down to 2s)
                        if consecutive_failures > 2 && current_interval_secs > 2 {
                            current_interval_secs = 2;
                            interval =
                                tokio::time::interval(Duration::from_secs(current_interval_secs));
                            interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
                        }

                        // Stop after too many failures
                        if consecutive_failures > 10 {
                            eprintln!(
                                "Keep-alive task stopping after {} failures",
                                consecutive_failures
                            );
                            break;
                        }

                        if consecutive_failures <= 3 {
                            eprintln!("Keep-alive request failed (#{consecutive_failures}): {e}");
                        }
                    }
                }
            }
        })
    }

    /// Get the number of currently available clients
    pub async fn available_count(&self) -> usize {
        // Number of available concurrency slots
        self.semaphore.available_permits()
    }

    /// Get the maximum number of concurrent requests
    pub fn max_concurrent(&self) -> usize {
        self.max_concurrent_requests
    }

    /// Get the number of active requests
    pub fn active_requests(&self) -> usize {
        self.metrics.active_requests.load(Ordering::Relaxed)
    }

    /// Get performance metrics
    pub fn metrics(&self) -> &RequestMetrics {
        &self.metrics
    }

    /// Reset all metrics to zero
    pub fn reset_metrics(&self) {
        self.metrics.total_requests.store(0, Ordering::Relaxed);
        self.metrics.successful_requests.store(0, Ordering::Relaxed);
        self.metrics.failed_requests.store(0, Ordering::Relaxed);
        self.metrics.active_requests.store(0, Ordering::Relaxed);
        self.metrics.peak_concurrent.store(0, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::ErrorKind;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::Mutex;
    use tokio::time::sleep;

    #[tokio::test]
    async fn test_req_manager_creation() {
        let manager = ReqManager::new(3).await.unwrap();
        assert_eq!(manager.max_concurrent(), 3);
        assert_eq!(manager.active_requests(), 0);
    }

    #[tokio::test]
    async fn test_req_manager_zero_clients() {
        let result = ReqManager::new(0).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_acquire_and_release() {
        let manager = ReqManager::new(2).await.unwrap();

        // Acquire first client
        let guard1 = manager.acquire().await.unwrap();
        assert_eq!(manager.active_requests(), 1);

        // Acquire second client
        let guard2 = manager.acquire().await.unwrap();
        assert_eq!(manager.active_requests(), 2);

        // Drop first guard to release client
        drop(guard1);

        // Give time for the async drop to complete
        sleep(Duration::from_millis(10)).await;

        // Should be able to acquire again
        let _guard3 = manager.acquire().await.unwrap();

        drop(guard2);
    }

    #[tokio::test]
    async fn test_concurrent_requests_limited() {
        let manager = Arc::new(ReqManager::new(3).await.unwrap());
        let counter = Arc::new(AtomicUsize::new(0));
        let max_concurrent = Arc::new(AtomicUsize::new(0));

        let mut handles = vec![];

        // Spawn 10 tasks that each try to acquire a client
        for _ in 0..10 {
            let manager = Arc::clone(&manager);
            let counter = Arc::clone(&counter);
            let max_concurrent = Arc::clone(&max_concurrent);

            let handle = tokio::spawn(async move {
                let _guard = manager.acquire().await.unwrap();

                // Increment active count
                let active = counter.fetch_add(1, Ordering::SeqCst) + 1;

                // Update max concurrent if needed
                max_concurrent.fetch_max(active, Ordering::SeqCst);

                // Simulate some work
                sleep(Duration::from_millis(50)).await;

                // Decrement active count
                counter.fetch_sub(1, Ordering::SeqCst);
            });

            handles.push(handle);
        }

        // Wait for all tasks to complete
        for handle in handles {
            handle.await.unwrap();
        }

        // Check that we never exceeded max concurrent requests
        let max_seen = max_concurrent.load(Ordering::SeqCst);
        assert!(
            max_seen <= 3,
            "Max concurrent was {max_seen} but should be <= 3"
        );
    }

    #[tokio::test]
    async fn test_client_guard_methods() {
        // Start a local test server
        let listener = match tokio::net::TcpListener::bind("127.0.0.1:0").await {
            Ok(listener) => listener,
            Err(err) => {
                if err.kind() == ErrorKind::PermissionDenied {
                    eprintln!("Skipping test_client_guard_methods: {err}");
                    return;
                }
                panic!("Failed to bind test server listener: {err}");
            }
        };
        let addr = listener.local_addr().unwrap();

        // Spawn a simple HTTP server
        tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                tokio::spawn(async move {
                    use tokio::io::AsyncWriteExt;
                    let response = "HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nOK";
                    let _ = socket.write_all(response.as_bytes()).await;
                });
            }
        });

        let manager = ReqManager::new(2).await.unwrap();
        let guard = manager.acquire().await.unwrap();

        // Test the client() method
        let client = guard.client();
        let url = format!("http://{addr}/test");
        let response = client.get(&url).send().await.unwrap();
        assert_eq!(response.status(), 200);

        // Test the get() helper method
        let guard2 = manager.acquire().await.unwrap();
        let response2 = guard2.get(&url).await.unwrap();
        assert_eq!(response2.status(), 200);
    }

    #[tokio::test]
    async fn test_warmup() {
        // Start a local test server for warmup
        let listener = match tokio::net::TcpListener::bind("127.0.0.1:0").await {
            Ok(listener) => listener,
            Err(err) => {
                if err.kind() == ErrorKind::PermissionDenied {
                    eprintln!("Skipping test_warmup: {err}");
                    return;
                }
                panic!("Failed to bind test warmup server: {err}");
            }
        };
        let addr = listener.local_addr().unwrap();

        // Use a channel to control server shutdown
        let (tx, mut rx) = tokio::sync::mpsc::channel::<()>(1);

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    accept_result = listener.accept() => {
                        if let Ok((mut socket, _)) = accept_result {
                            tokio::spawn(async move {
                                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                                let mut buf = [0; 1024];
                                // Read the request
                                let _ = socket.read(&mut buf).await;
                                // Send response
                                let response = "HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n";
                                let _ = socket.write_all(response.as_bytes()).await;
                                let _ = socket.flush().await;
                            });
                        }
                    }
                    _ = rx.recv() => {
                        break;
                    }
                }
            }
        });

        // Give server time to start
        sleep(Duration::from_millis(10)).await;

        let manager = ReqManager::new(3).await.unwrap();
        let url = format!("http://{addr}/warmup");

        // Test warmup with OPTIONS
        manager.warmup(&url, "OPTIONS").await.unwrap();

        // Small delay between warmups
        sleep(Duration::from_millis(10)).await;

        // Test warmup with GET
        manager.warmup(&url, "GET").await.unwrap();

        // Cleanup
        let _ = tx.send(()).await;
    }

    #[tokio::test]
    async fn test_aggressive_warmup() {
        // Start a local test server for aggressive warmup
        let listener = match tokio::net::TcpListener::bind("127.0.0.1:0").await {
            Ok(listener) => listener,
            Err(err) => {
                if err.kind() == ErrorKind::PermissionDenied {
                    eprintln!("Skipping test_aggressive_warmup: {err}");
                    return;
                }
                panic!("Failed to bind test aggressive warmup server: {err}");
            }
        };
        let addr = listener.local_addr().unwrap();

        // Use a channel to control server shutdown
        let (tx, mut rx) = tokio::sync::mpsc::channel::<()>(1);

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    accept_result = listener.accept() => {
                        if let Ok((mut socket, _)) = accept_result {
                            tokio::spawn(async move {
                                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                                let mut buf = [0; 1024];
                                // Read the request
                                let _ = socket.read(&mut buf).await;
                                // Send response
                                let response = "HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n";
                                let _ = socket.write_all(response.as_bytes()).await;
                                let _ = socket.flush().await;
                            });
                        }
                    }
                    _ = rx.recv() => {
                        break;
                    }
                }
            }
        });

        // Give server time to start
        sleep(Duration::from_millis(10)).await;

        let manager = ReqManager::new(3).await.unwrap();
        let url = format!("http://{addr}/warmup");

        // Test aggressive warmup
        manager.aggressive_warmup(&url).await.unwrap();

        // Verify manager is still functional after aggressive warmup
        let guard = manager.acquire().await.unwrap();
        assert!(guard.client().get(&url).send().await.is_ok());

        // Cleanup
        let _ = tx.send(()).await;
    }

    #[tokio::test]
    async fn test_queuing_behavior() {
        let manager = Arc::new(ReqManager::new(2).await.unwrap());
        let order = Arc::new(Mutex::new(Vec::new()));

        // Hold both clients
        let guard1 = manager.acquire().await.unwrap();
        let guard2 = manager.acquire().await.unwrap();

        // Start tasks that will queue for clients
        let mut handles = vec![];
        for i in 0..3 {
            let manager = Arc::clone(&manager);
            let order = Arc::clone(&order);

            let handle = tokio::spawn(async move {
                let _guard = manager.acquire().await.unwrap();
                let mut vec = order.lock().await;
                vec.push(i);
                sleep(Duration::from_millis(10)).await;
            });

            handles.push(handle);
        }

        // Give time for tasks to queue
        sleep(Duration::from_millis(50)).await;

        // Release clients
        drop(guard1);
        drop(guard2);

        // Wait for queued tasks to complete
        for handle in handles {
            handle.await.unwrap();
        }

        // Verify all tasks completed
        let final_order = order.lock().await;
        assert_eq!(final_order.len(), 3);
    }
}
