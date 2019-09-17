// TODO: Store like three minutes of data, then change how much is shown based on scaling!
pub const STALE_MAX_MILLISECONDS : u64 = 60 * 1000; // We wish to store at most 60 seconds worth of data.  This may change in the future, or be configurable.
pub const TICK_RATE_IN_MILLISECONDS : u64 = 200; // We use this as it's a good value to work with.