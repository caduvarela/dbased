# Development databases

Postgres and MySQL with sample data, for exploring the app by hand (explorer, grid, editing). Never used by automated tests, which use testcontainers. Credentials are throwaway and dev-only.

Image versions and host ports are set in the repo-root `.env`.

## Usage

```
docker compose up -d       # start (first start seeds, ~30 s)
docker compose ps          # wait for "healthy"
docker compose down        # stop, keep data
docker compose down -v     # stop and delete data (reset; reseeds on next up)
```

Seed scripts run only on an empty volume. Edit them, then `down -v` and `up -d`.

## Connections (bound to 127.0.0.1)

| Engine   | Host port | Database    | User     | Password      |
|----------|-----------|-------------|----------|---------------|
| Postgres | 54320     | `dbased_dev`| `dbased` | `dbased`      |
| MySQL    | 33060     | `dbased_dev`| `dbased` | `dbased`      |

MySQL root password: `dbased-root`.

## Seed contents

Postgres schemas `public`, `sales`, `inventory`. MySQL equivalents are databases `dbased_dev`, `dbased_sales`, `dbased_inventory`.

- 10 tables per engine with primary keys, foreign keys and secondary indexes.
- `audit_log`: no primary key and no unique index (read-only-by-default editing tier).
- Composite primary keys: `sales.order_items` and `inventory.stock` (Postgres); `dbased_sales.order_items` and `dbased_inventory.stock` (MySQL).
- `events`: 1,000,000 rows, for scrolling and keyset pagination.
- Two views (`order_totals`, `low_stock`), a function (`customer_spend`) and a procedure (`restock`) in each engine.
- Postgres types: enum, jsonb, arrays, uuid, numeric, timestamptz.
- MySQL types: decimal, json, enum, set, bit, `datetime(3)`, and zero dates in `legacy_dates`.
