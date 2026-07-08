# awl website — static site (landing + the /editor wasm bundle) served by Caddy.
# Built + deployed on Fly.io from the site/ dir (this is the build context).
# Caddy serves /srv on :8080 with zstd/gzip compression (the ~28 MB wasm
# compresses to ~9 MB on the wire) and the correct application/wasm MIME.
FROM caddy:2-alpine
COPY Caddyfile /etc/caddy/Caddyfile
COPY . /srv
