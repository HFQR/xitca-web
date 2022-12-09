#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: (Vec<(String, i32)>, String, Option<bool>)| {
    let mut matcher = matchit::Node::new();

    for (key, item) in data.0 {
        if matcher.insert(key, item).is_err() {
            return;
        }
    }

    match data.2 {
        None => {
            let _ = matcher.at(&data.1);
        }
        Some(b) => {
            let _ = matcher.path_ignore_case(&data.1, b);
        }
    }
});
