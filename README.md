<h1 align="center">Xitca-Web</h1>
<p align="center">
  <b align="center"><a href="README.md">Readme</a></b> |
  <b><a href="https://github.com/HFQR/xitca-web">GitHub</a></b> |
  <b><a href="https://docs.rs/xitca-web/latest/xitca_web/">Documentation</a></b>
  <br /><br />
  <a href="#">
    <img
      alt="GitHub code size in bytes"
      src="https://img.shields.io/github/languages/code-size/HFQR/xitca-web?style=flat-square"
    />
  </a>
  <a href=""
    ><img
      alt="Maintenance"
      src="https://img.shields.io/maintenance/yes/2024?style=flat-square"
    />
  </a>
  <br />
  <br />
  <i>
    A HTTP library and web framework written in 100% safe Rust with zero-copy
    serialization/deserilization support
  </i>
</p>

<br/>

<details>
  <summary><b>Table of Contents</b></summary>
  <p>

  - [Features](#features)
  - [Minimum Supported Rust Version](#minimum-supported-rust-version)
  - [Quick Start](#quick-start)
  - [FAQ (Frequently Asked Questions)](#faq-frequently-asked-questions)
  - [More Contributors Wanted](#more-contributors-wanted)
  - [Supporting Xitca-web](#supporting-xitca-web)
  - [Documentation](#documentation)
  - [Roadmap](#roadmap)
  - [Contributing](#contributing)
  - [License](#license)
  - [Credits](#credits)

  </p>
</details>

# Features

- Supports HTTP/1.x, HTTP/2 and HTTP/3.
- Powerful request routing with optional opt-in macros
- Full Tokio compatibility
- Cross crate integration with Tower.
- Client/server WebSockets support
- Transparent content compression/decompression (br, gzip, deflate, zstd)
- Multipart streams
- Static assets
- SSL support using Xitca-tls or rustls
- Middlewares (Logger, Tracing, etc)

**[⬆️ Back to Top](#xitca-web)**

# Minimum Supported Rust Version

The latest release of the crate supports 1.85 and above rust versions.

**[⬆️ Back to Top](#xitca-web)**

# Quick Start

> For a list of other examples, see: [**Examples**](examples)

To get started using Xitca-web add the following code to your `main.rs` file: 
```rust
use xitca_web::{handler::handler_service, middleware::Logger, route::get, App};

async fn index() -> &'static str {
    "Hello world!!"
}

fn main() -> std::io::Result<()> {
    App::new()
        .at("/", get(handler_service(index)))
        .enclosed(Logger::new())
        .serve()
        .bind("localhost:8080")?
        .run()
        .wait()
}
```

And then add the following dependencies to your `Cargo.toml` file:

```toml
xitca-web = { version = "*" , features = ["logger"]}
```

And then run the application/server using `cargo run` command and it should start the server/application on <http://127.0.0.1:8080>. Open your prefered browser and navigate to the address on which the server has started and you should see a `hello world!!` message appear.

> [!Warning]
> The project is still not stable. So expect breaking changes to appear at any time.

**[⬆️ Back to Top](#xitca-web)**

# FAQ (Frequently Asked Questions)

## Why Yet Another HTTP Library and Framework?

The primary reason behind creating Xitca-web is to provide more memory safe framework which uses 100% safe rust. As well as to provide a framework which provides its own HTTP library, async IO abstraction, tls integration and a database driver all in one package with a low-memory footprint, less synchronization overhead between threads, a very small dependency tree and with good interoperability between with other async frameworks like `tokio`, `async-std`, etc and with crates like Tower and HTTP. Additionally, the following HTTP library and framework was built to provide a simple and compact code base with easier to understand and adoptable API both internally and externally and to provide the choice to the user to opt-out of macro usage when using `xitca-web` which can greatly improve build/compile times by eliminating the extra `proc-macro` build/compile step.

**[⬆️ Back to Top](#xitca-web)**

# More Contributors Wanted

We are looking for more willing contributors to help grow this project. For more information on how you can contribute, check out the [project board](https://github.com/HFQR/xitca-web/projects?query=is%3Aopen) and the [CONTRIBUTING.md](CONTRIBUTING.md) file for guidelines and rules for making contributions.

**[⬆️ Back to Top](#xitca-web)**

# Supporting Xitca-web

> For full details and other ways you can help out, see: [**Contributing**](CONTRIBUTING.md)

If you use Xitca-web and would like to contribute to its development, we're glad to have you on board! Contributions of any size or type are always welcome, and we will always acknowledge your efforts.

Several areas that we need a bit of help with at the moment are:

- **Documentation**: Help improve and write documentation for different features.
- **Logo**: Help create a logo for the project.
- Submit a PR to add a new feature, fix a bug, update the docs, or anything else.
- Star Xitca-web on GitHub.

**[⬆️ Back to Top](#xitca-web)**

# Documentation

> [!Note]
> We welcome any contributions to the [documentation](https://docs.rs/xitca-web/latest/xitca_web/) as this will benefit everyone who uses this project.

**[⬆️ Back to Top](#xitca-web)**

# Roadmap

> Coming soon!.

**[⬆️ Back to Top](#xitca-web)**

# Contributing

Contributions are welcome from anyone. It doesn't matter who you are; you can still contribute to the project in your own way.

## Not a developer but still want to contribute?

Check out this [video](https://youtu.be/FccdqCucVSI) by Mr. Nick on how to contribute.

## Developer

If you are a developer, have a look at the [CONTRIBUTING.md](CONTRIBUTING.md) document for more information.

**[⬆️ Back to Top](#xitca-web)**

# License

Xitca-web is licensed under the [APACHEv2](LICENSE) license.

**[⬆️ Back to Top](#xitca-web)**

# Credits

We would like to thank the following people for their contributions and support:

**Contributors**

<p>
  <br />
  <a href="https://github.com/HFQR/xitca-web/graphs/contributors">
    <img src="https://contrib.rocks/image?repo=HFQR/xitca-web" />
  </a>
  <br />
</p>

**Stargazers**

<p>
  <a href="https://github.com/HFQR/xitca-web/stargazers">
    <img src="http://reporoster.com/stars/dark/HFQR/xitca-web"/>
  </a>
</p>

**[⬆️ Back to Top](#xitca-web)**

---

<p align="center">
  <a href="https://github.com/HFQR/xitca-web">
    <img src="https://github.githubassets.com/images/icons/emoji/octocat.png" />
  </a>
  <br /><br />
  <i>Thank you for Visiting</i>
</p>
