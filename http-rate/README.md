# rate limit for http types

```rust
use std::net::SocketAddr;

use http::{Request, Response};
use http_rate::{Quota, RateLimit};

// a quota limiting request to 1 per second.
let quota = Quota::per_second(1);

// construct rate limiter with given quota.
let limiter = RateLimit::new(quota);

async fn request(lim: &RateLimit, req: &Request<()>, addr: SocketAddr) -> Response<()> {
    // rate limiter needs request header map and client socket addr.
    match lim.rate_limit(req.headers(), &addr) {
        // client still have quota left.
        Ok(snap) => {
            // rate limiter guarded logic can be executed here. 
            // in this case a dummy response is constructed.
            let mut res = Response::new(());

            // snapshot return from rate_limit method can be used to extend
            // rate-limit related headers to response.
            snap.extend_response(&mut res);

            res
        }
        // client ran out of quota.
        Err(e) => {
            // logic not guarded by rate limiter can be executed here.
            // in this case a dummy response is constructed.
            let mut res = Response::new(());

            // error return from rate_limit method can be used to extend rate-limit
            // related headers and status code to error response.
            e.extend_response(&mut res);

            // error can be formatted.
            println!("{e:?}");
            println!("{e}");

            // error can be boxed as std::error::Error trait object.
            let _ = Box::new(e) as Box<dyn std::error::Error + Send + Sync>;

            res
        }
    }
}
```
