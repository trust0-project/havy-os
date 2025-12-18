//! Real-Time Clock (RTC) Support
//!
//! Reads host timestamp from RTC MMIO device at 0x10100000.
//! The VM must provide the Unix timestamp at this address.

use core::ptr;

/// RTC MMIO base address
const RTC_BASE: usize = 0x10100000;

/// Read host Unix timestamp (seconds since 1970-01-01 00:00:00 UTC)
/// Returns 0 if RTC is not available
pub fn get_host_timestamp() -> u64 {
    unsafe {
        let low = ptr::read_volatile((RTC_BASE) as *const u32) as u64;
        let high = ptr::read_volatile((RTC_BASE + 4) as *const u32) as u64;
        (high << 32) | low
    }
}

/// Simple date/time representation
#[derive(Clone, Copy)]
pub struct DateTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

impl DateTime {
    /// Convert Unix timestamp to DateTime (UTC)
    pub fn from_unix(timestamp: u64) -> Self {
        // Days since epoch
        let mut days = (timestamp / 86400) as i64;
        let day_seconds = (timestamp % 86400) as u32;
        
        let hour = (day_seconds / 3600) as u8;
        let minute = ((day_seconds % 3600) / 60) as u8;
        let second = (day_seconds % 60) as u8;
        
        // Calculate year (start from 1970)
        let mut year = 1970i32;
        loop {
            let days_in_year = if is_leap_year(year) { 366 } else { 365 };
            if days < days_in_year {
                break;
            }
            days -= days_in_year;
            year += 1;
        }
        
        // Calculate month and day
        let leap = is_leap_year(year);
        let days_in_months: [i64; 12] = if leap {
            [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
        } else {
            [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
        };
        
        let mut month = 0u8;
        for (i, &dim) in days_in_months.iter().enumerate() {
            if days < dim {
                month = (i + 1) as u8;
                break;
            }
            days -= dim;
        }
        
        let day = (days + 1) as u8; // Days are 1-indexed
        
        Self {
            year: year as u16,
            month,
            day,
            hour,
            minute,
            second,
        }
    }
}

fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

/// Get current host date/time
/// Returns None if RTC is not available (timestamp is 0)
pub fn get_datetime() -> Option<DateTime> {
    let ts = get_host_timestamp();
    if ts == 0 {
        None
    } else {
        Some(DateTime::from_unix(ts))
    }
}
