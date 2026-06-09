---
layout: default
title: Jjinx docs
nav_order: 1
---

# Jjinx docs

## Building
```bash
cargo build --release
```

## Usage
```bash
jjinx --config your-config.conf
```

# Configuration format
jjinx uses a similiar to nginx config format, but with some differences

## Example
```
upstream backend {
    round_robin
    server app1.example.com
    server app2.example.com
}

server {
    port 8080
    default

    route = / {
        redirect /index.html
    }

    route /index.html {
        file index.html
    }

    route /get-host {
        body "Host: $host\n"
    }

    route /simple-proxy {
        proxy http://app.example.com:80
    }

    route /upstream-proxy/ {
        proxy http://backend:8000
    }

    route /404 {
        body "Not found"
    }

    error_page 404 /404
}

server {
    port 4433
    default

    ssl_cert server.crt
    ssl_keys server.key

    route / {
        proxy http://localhost:8080
    }
}

```