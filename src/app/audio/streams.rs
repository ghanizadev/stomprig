use std::sync::{Arc, Mutex};

pub fn report_error(target: &Arc<Mutex<Option<String>>>, message: String) {
    if let Ok(mut error) = target.lock() {
        *error = Some(message);
    }
}
