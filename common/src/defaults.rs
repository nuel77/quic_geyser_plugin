pub const DEFAULT_MAX_STREAMS: u64 = 128;
pub const DEFAULT_MAX_RECIEVE_WINDOW_SIZE: u64 = 24 * 1024 * 1024; // 24 MBs
pub const DEFAULT_CONNECTION_TIMEOUT: u64 = 10;
pub const DEFAULT_MAX_NB_CONNECTIONS: u64 = 10;
pub const DEFAULT_MAX_ACK_DELAY: u64 = 25;
pub const DEFAULT_ACK_EXPONENT: u64 = 3;
pub const ALPN_GEYSER_PROTOCOL_ID: &[u8] = b"geyser";
pub const MAX_DATAGRAM_SIZE: usize = 1350;
pub const MAX_PAYLOAD_BUFFER: usize = 5 * MAX_DATAGRAM_SIZE;
pub const DEFAULT_ENABLE_PACING: bool = true;
pub const DEFAULT_CC_ALGORITHM: &str = "cubic";
pub const DEFAULT_INCREMENTAL_PRIORITY: bool = true;
pub const DEFAULT_ENABLE_GSO: bool = true;
pub const DEFAULT_DISCOVER_PMTU: bool = true;
pub const DEFAULT_PARALLEL_STREAMS: usize = 32;
pub const DEFAULT_DISCONNECT_LAGGY_CLIENTS: bool = true;
