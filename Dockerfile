FROM ruby:3.1 AS docs-builder
WORKDIR /app

COPY docs/Gemfile docs/Gemfile.lock ./
RUN bundle install

COPY docs/_config.yml docs/index.md ./
RUN bundle exec jekyll build

FROM rust:1.94 AS builder
WORKDIR /app

COPY src ./src
COPY Cargo.toml ./
RUN cargo build --release

COPY --from=docs-builder /app/_site ./_site
COPY presentation _site/presentation
COPY docs/jjinx.conf ./jjinx.conf

EXPOSE 8080
ENTRYPOINT ["./target/release/jjinx", "--config", "jjinx.conf"]
