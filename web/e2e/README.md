# Production user-credential acceptance

`production-user-credentials.test.mjs` uses the deployed UI and real backend. It does not intercept API requests. Run it from `web/` with Node.js 24 and `playwright-core` plus Chromium available:

```sh
E2E_BASE_URL=https://workspace.example.org \
E2E_ADMIN_TOKEN="$ADMIN_TOKEN" \
E2E_ORGANIZATION_ID="$ORGANIZATION_ID" \
E2E_TARGET_USER_ID="$ACCEPTANCE_USER_ID" \
node --test e2e/production-user-credentials.test.mjs
```

The token must belong to an unrestricted system administrator with `manage_system` and `manage_api_keys`. The target must be a different, enabled user. By default the test only reads; set `E2E_ALLOW_WRITES=1` to create a short-lived key through the UI, verify copy/list readback, and revoke it. The test also attempts revocation during cleanup if an assertion fails. Do not use a person's account as the target. `CHROMIUM_BIN` and `E2E_PLAYWRIGHT_MODULE` can point to an existing browser/runtime in restricted environments.
