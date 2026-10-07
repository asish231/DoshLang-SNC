# SNlang starter & docs site (React + Vite)

The SNlang language starter and documentation UI served by `examples/website/server.sn`:
Home, Get Started, a 14-chapter language guide, a live playground (runs code via
`/api/sandbox/run`), the standard-library reference, a repo examples browser, and the
tooling reference.

Every runnable snippet lives in `snippets/*.sn` and is compile+run verified:

```sh
node scripts/verify-snippets.mjs   # compiles + runs each snippet with ../../../../snc
npm run lint                        # oxlint
npm run build                       # vite build -> dist/
```

The SNlang server serves `examples/website/public/`, so after building, sync the
output there (filenames are content-hashed; drop the stale bundle first):

```sh
rm -f ../public/assets/* && cp dist/index.html ../public/index.html && cp dist/assets/* ../public/assets/
```

Dev mode with API proxy to the SNlang backend on :8090:

```sh
npm run dev
```
