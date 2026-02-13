#!/bin/sh
set -e

node - <<'NODE'
const fs = require('fs');

const config = {
  apiBase: process.env.API_BASE_URL || '/api',
};

fs.writeFileSync(
  '/app/dist/runtime-config.js',
  `window.__CLEO_RUNTIME_CONFIG__ = ${JSON.stringify(config)};\n`
);
NODE

exec "$@"
