use reqwest::{Client, Response};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Semaphore, SemaphorePermit};

/// A connection pool manager that maintains a pool of pre-warmed HTTP/2 clients
/// with proper concurrency control and request queuing.
///
/// # Example
/// ```rust,no_run
/// # async fn example() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
/// use sayna::utils::req_manager::ReqManager;
///
/// let manager = ReqManager::new(5).await?;
///
/// // Get a client and make a request
/// let guard = manager.acquire().await?;
/// let response = guard.client()
///     .get("https://api.example.com/data")
///     .send()
///     .await?;
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
}

/// A guard that holds a client from the pool and returns it when dropped
pub struct ClientGuard<'a> {
    _manager: &'a ReqManager,
    client: Arc<Client>,
    _permit: SemaphorePermit<'a>,
}

impl<'a> ClientGuard<'a> {
    /// Get the HTTP client for making requests
    pub fn client(&self) -> &Client {
        &self.client
    }

    /// Make a GET request
    pub async fn get(&self, url: &str) -> Result<Response, reqwest::Error> {
        self.client.get(url).send().await
    }

    /// Make a POST request
    pub async fn post(&self, url: &str) -> Result<Response, reqwest::Error> {
        self.client.post(url).send().await
    }
}

impl ReqManager {
    /// Create a new request manager with the specified maximum concurrent requests
    ///
    /// # Arguments
    /// * `max_concurrent_requests` - Maximum number of concurrent requests allowed
    ///
    /// # Returns
    /// A new `ReqManager` instance with pre-warmed connections
    pub async fn new(
        max_concurrent_requests: usize,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        if max_concurrent_requests == 0 {
            return Err("max_concurrent_requests must be greater than 0".into());
        }

        // Single optimized client with pooling and HTTP/2 multiplexing
        let client = Arc::new(Self::create_optimized_client()?);

        Ok(Self {
            max_concurrent_requests,
            client,
            semaphore: Arc::new(Semaphore::new(max_concurrent_requests)),
        })
    }

    /// Create an optimized HTTP/2 client with connection pooling
    fn create_optimized_client() -> Result<Client, reqwest::Error> {
        Client::builder()
            // HTTP/2 configuration for multiplexing (will fallback to HTTP/1.1 if not supported)
            // Do not use http2_prior_knowledge to preserve broader compatibility
            .http2_initial_stream_window_size(4_194_304) // 4MB for faster data transfer
            .http2_initial_connection_window_size(8_388_608) // 8MB for better throughput
            .http2_keep_alive_interval(Some(Duration::from_secs(5))) // More frequent keep-alive
            .http2_keep_alive_timeout(Duration::from_secs(10))
            .http2_keep_alive_while_idle(true)
            .http2_adaptive_window(true)
            // Connection pool configuration tuned for low-latency reuse
            .pool_idle_timeout(None) // keep idle connections indefinitely
            .pool_max_idle_per_host(256) // Increased from 128 for better connection reuse
            // TCP optimizations
            .tcp_keepalive(Duration::from_secs(10)) // More aggressive keep-alive
            .tcp_nodelay(true) // Disable Nagle's algorithm for lower latency
            .connect_timeout(Duration::from_millis(1000)) // Reduced from 5s for faster failures
            .timeout(Duration::from_secs(30))
            .user_agent("sayna-req-manager/1.0")
            .build()
    }

    /// Acquire a client from the pool
    ///
    /// This method will wait if all clients are currently in use.
    /// The returned `ClientGuard` will automatically return the client
    /// to the pool when dropped.
    ///
    /// # Returns
    /// A `ClientGuard` that provides access to an HTTP client
    pub async fn acquire(
        &self,
    ) -> Result<ClientGuard<'_>, Box<dyn std::error::Error + Send + Sync>> {
        // Acquire semaphore permit to ensure we don't exceed max concurrent requests
        let permit = self.semaphore.acquire().await?;

        let client = Arc::clone(&self.client);

        Ok(ClientGuard {
            _manager: self,
            client,
            _permit: permit,
        })
    }

    /// Warm up connections by sending lightweight requests
    ///
    /// # Arguments
    /// * `url` - The URL to warm up connections with
    /// * `warmup_type` - Type of warmup: "HEAD", "OPTIONS", or "GET"
    pub async fn warmup(
        &self,
        url: &str,
        warmup_type: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        use futures::future::join_all;

        // Use multiple concurrent lightweight requests to prime TLS, ALPN, and pools.
        let parallel_streams = self.max_concurrent_requests.clamp(1, 8);

        let warmup_futures: Vec<_> = (0..parallel_streams)
            .map(|i| {
                let client = Arc::clone(&self.client);
                let url = url.to_string();
                let warmup_type = warmup_type.to_string();

                async move {
                    let start = std::time::Instant::now();

                    let result = match warmup_type.as_str() {
                        "HEAD" => client.head(&url).send().await,
                        "OPTIONS" => client.request(reqwest::Method::OPTIONS, &url).send().await,
                        "GET" => client.get(&url).send().await,
                        _ => client.request(reqwest::Method::OPTIONS, &url).send().await,
                    };

                    match result {
                        Ok(resp) => {
                            // Consume response to properly reuse connection
                            let _ = resp.bytes().await;
                            let elapsed = start.elapsed();
                            println!("  Warmup stream {i} completed in {elapsed:?}");
                            Ok(())
                        }
                        Err(e) => {
                            eprintln!("  Warmup stream {i} failed: {e}");
                            Err(e)
                        }
                    }
                }
            })
            .collect();

        let results = join_all(warmup_futures).await;

        for result in results {
            result?;
        }

        println!("Warmup complete for {parallel_streams} parallel streams");
        Ok(())
    }

    /// Aggressive warmup by sending 2x max concurrent requests to fully prime the connection pool
    ///
    /// This method sends multiple concurrent HEAD requests to establish and warm up
    /// all potential connections in the pool, reducing first-request latency.
    ///
    /// # Arguments
    /// * `url` - The URL to warm up connections with
    pub async fn aggressive_warmup(
        &self,
        url: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        use futures::future::join_all;

        // Send 2x concurrent requests to fully warm the pool
        let warmup_count = self.max_concurrent_requests * 2;

        println!(
            "Starting aggressive warmup with {} parallel requests",
            warmup_count
        );

        let warmup_futures: Vec<_> = (0..warmup_count)
            .map(|i| {
                let client = Arc::clone(&self.client);
                let url = url.to_string();

                async move {
                    let start = std::time::Instant::now();

                    // Use HEAD requests with short timeout for minimal overhead
                    let result = client
                        .head(&url)
                        .timeout(Duration::from_millis(200))
                        .send()
                        .await;

                    match result {
                        Ok(resp) => {
                            // Consume response to properly reuse connection
                            let _ = resp.bytes().await;
                            let elapsed = start.elapsed();
                            if i < 10 {
                                // Only log first 10 to avoid spam
                                println!(
                                    "  Aggressive warmup stream {} completed in {:?}",
                                    i, elapsed
                                );
                            }
                            Ok(())
                        }
                        Err(e) if e.is_timeout() => {
                            // Timeout is OK for warmup, connection is established
                            Ok(())
                        }
                        Err(e) => {
                            if i < 10 {
                                // Only log first 10 errors
                                eprintln!("  Aggressive warmup stream {} failed: {}", i, e);
                            }
                            Err(e)
                        }
                    }
                }
            })
            .collect();

        let results = join_all(warmup_futures).await;

        let successful = results.iter().filter(|r| r.is_ok()).count();
        println!(
            "Aggressive warmup complete: {}/{} successful",
            successful, warmup_count
        );

        Ok(())
    }

    /// Start a background task that keeps connections alive by sending periodic lightweight requests
    ///
    /// This prevents connection closure due to idle timeouts and ensures connections
    /// remain warm for immediate use.
    ///
    /// # Arguments
    /// * `url` - The URL to send keep-alive requests to (should be a lightweight endpoint)
    ///
    /// # Returns
    /// A JoinHandle for the spawned task that can be used to cancel the keep-alive loop
    pub fn start_keepalive_task(&self, url: String) -> tokio::task::JoinHandle<()> {
        let client = Arc::clone(&self.client);
        let interval_secs = 5; // Send keep-alive every 5 seconds

        println!("Starting keep-alive task for URL: {}", url);

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(interval_secs));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

            loop {
                interval.tick().await;

                // Send lightweight HEAD request to keep connection alive
                let result = client
                    .head(&url)
                    .header("X-Keep-Alive", "true")
                    .timeout(Duration::from_millis(100))
                    .send()
                    .await;

                match result {
                    Ok(_) => {
                        // Success - connection kept alive
                    }
                    Err(e) if e.is_timeout() => {
                        // Timeout is acceptable for keep-alive
                    }
                    Err(e) => {
                        // Log error but continue - connection might recover
                        eprintln!("Keep-alive request failed: {}", e);
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
        self.max_concurrent_requests - self.semaphore.available_permits()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
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
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
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
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
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
