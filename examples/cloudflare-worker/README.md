# cloudflare worker with xitca-web

## Requirement
- nightly Rust
- [cloudflare workers](https://workers.cloudflare.com/)

## API
Same as auto generated worker template with additional static stie.
```
GET  /
POST /form/<field_name_string> 
GET  /worker-version
```

## Example site
https://xitca-web-worker.fakeshadow.workers.dev/

## Usage
```bash
# install wrangler cli.
npm install

# run your Worker in an ideal development workflow (with a local server, file watcher & more)
npm run dev

# deploy your Worker globally to the Cloudflare network (update your wrangler.toml file for configuration)
npm run deploy
```
Read the latest `worker` crate documentation here: https://docs.rs/worker
