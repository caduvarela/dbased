-- Dev-only seed for manual exploration. Runs once, on first start of an empty volume.
\set ON_ERROR_STOP on

CREATE EXTENSION IF NOT EXISTS pgcrypto;

CREATE SCHEMA IF NOT EXISTS sales;
CREATE SCHEMA IF NOT EXISTS inventory;

CREATE TYPE public.order_status AS ENUM ('pending', 'paid', 'shipped', 'cancelled');

-- public ---------------------------------------------------------------

CREATE TABLE public.customers (
    id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    name        text NOT NULL,
    email       text NOT NULL UNIQUE,
    tags        text[] NOT NULL DEFAULT '{}',
    profile     jsonb NOT NULL DEFAULT '{}',
    created_at  timestamptz NOT NULL DEFAULT now()
);
COMMENT ON TABLE public.customers IS 'Customers with uuid key, array and jsonb columns.';

CREATE TABLE public.app_settings (
    key    text PRIMARY KEY,
    value  jsonb NOT NULL
);

-- No primary key and no unique index: exercises the read-only-by-default tier.
CREATE TABLE public.audit_log (
    happened_at  timestamptz NOT NULL DEFAULT now(),
    actor        text,
    action       text NOT NULL,
    details      jsonb
);

-- sales ----------------------------------------------------------------

CREATE TABLE sales.orders (
    id           bigserial PRIMARY KEY,
    customer_id  uuid NOT NULL REFERENCES public.customers (id),
    status       public.order_status NOT NULL DEFAULT 'pending',
    total        numeric(12, 2) NOT NULL DEFAULT 0,
    placed_at    timestamptz NOT NULL DEFAULT now(),
    notes        text
);
CREATE INDEX orders_customer_idx ON sales.orders (customer_id);
CREATE INDEX orders_status_placed_idx ON sales.orders (status, placed_at DESC);

CREATE TABLE inventory.products (
    id        serial PRIMARY KEY,
    sku       text NOT NULL UNIQUE,
    name      text NOT NULL,
    price     numeric(12, 2) NOT NULL,
    attrs     jsonb NOT NULL DEFAULT '{}',
    active    boolean NOT NULL DEFAULT true
);

-- Composite primary key.
CREATE TABLE sales.order_items (
    order_id    bigint NOT NULL REFERENCES sales.orders (id) ON DELETE CASCADE,
    product_id  integer NOT NULL REFERENCES inventory.products (id),
    quantity    integer NOT NULL CHECK (quantity > 0),
    unit_price  numeric(12, 2) NOT NULL,
    PRIMARY KEY (order_id, product_id)
);
CREATE INDEX order_items_product_idx ON sales.order_items (product_id);

CREATE TABLE inventory.warehouses (
    id    serial PRIMARY KEY,
    name  text NOT NULL,
    city  text
);

CREATE TABLE inventory.stock (
    warehouse_id  integer NOT NULL REFERENCES inventory.warehouses (id),
    product_id    integer NOT NULL REFERENCES inventory.products (id),
    on_hand       integer NOT NULL DEFAULT 0,
    updated_at    timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (warehouse_id, product_id)
);

CREATE TABLE inventory.suppliers (
    id       serial PRIMARY KEY,
    name     text NOT NULL,
    contact  jsonb
);

-- Large table for scrolling and keyset pagination (~1,000,000 rows).
CREATE TABLE public.events (
    id          bigserial PRIMARY KEY,
    customer_id uuid,
    kind        text NOT NULL,
    amount      numeric(12, 2) NOT NULL,
    payload     jsonb NOT NULL,
    occurred_at timestamptz NOT NULL
);

-- Data -----------------------------------------------------------------

INSERT INTO public.customers (name, email, tags, profile, created_at)
SELECT 'Customer ' || g,
       'customer' || g || '@example.test',
       ARRAY['tag' || (g % 5), 'group' || (g % 3)],
       jsonb_build_object('age', 18 + g % 60, 'vip', g % 10 = 0, 'address', jsonb_build_object('city', 'City ' || g % 20)),
       now() - (g || ' hours')::interval
FROM generate_series(1, 200) AS g;

INSERT INTO public.app_settings VALUES
    ('theme', '{"mode": "dark"}'),
    ('limits', '{"rows": 1000, "timeout_s": 30}');

INSERT INTO public.audit_log (actor, action, details)
SELECT 'user' || g % 7, (ARRAY['login', 'logout', 'update', 'delete'])[1 + g % 4], jsonb_build_object('n', g)
FROM generate_series(1, 50) AS g;

INSERT INTO inventory.products (sku, name, price, attrs)
SELECT 'SKU-' || lpad(g::text, 5, '0'), 'Product ' || g, round((random() * 500 + 1)::numeric, 2),
       jsonb_build_object('color', (ARRAY['red', 'green', 'blue'])[1 + g % 3], 'weight_kg', g % 9 + 0.5)
FROM generate_series(1, 100) AS g;

INSERT INTO inventory.warehouses (name, city) VALUES
    ('Main', 'Sao Paulo'), ('North', 'Recife'), ('South', 'Porto Alegre');

INSERT INTO inventory.stock (warehouse_id, product_id, on_hand)
SELECT w.id, p.id, (w.id * p.id) % 200
FROM inventory.warehouses w CROSS JOIN inventory.products p;

INSERT INTO inventory.suppliers (name, contact)
SELECT 'Supplier ' || g, jsonb_build_object('email', 'sales@supplier' || g || '.test')
FROM generate_series(1, 15) AS g;

INSERT INTO sales.orders (customer_id, status, total, placed_at)
SELECT (SELECT id FROM public.customers ORDER BY id OFFSET (g % 200) LIMIT 1),
       (ARRAY['pending', 'paid', 'shipped', 'cancelled'])[1 + g % 4]::public.order_status,
       0,
       now() - (g || ' hours')::interval
FROM generate_series(1, 2000) AS g;

INSERT INTO sales.order_items (order_id, product_id, quantity, unit_price)
SELECT o.id, p.id, 1 + (o.id + p.id) % 5, p.price
FROM sales.orders o
JOIN inventory.products p ON p.id IN (1 + o.id % 100, 1 + (o.id * 7) % 100, 1 + (o.id * 13) % 100);

UPDATE sales.orders o
SET total = s.total
FROM (SELECT order_id, sum(quantity * unit_price) AS total FROM sales.order_items GROUP BY order_id) s
WHERE s.order_id = o.id;

INSERT INTO public.events (customer_id, kind, amount, payload, occurred_at)
SELECT NULL,
       (ARRAY['click', 'view', 'purchase', 'refund'])[1 + g % 4],
       round((g % 10000) / 100.0, 2),
       jsonb_build_object('seq', g, 'ok', g % 3 <> 0),
       timestamptz '2024-01-01 00:00:00+00' + (g || ' seconds')::interval
FROM generate_series(1, 1000000) AS g;
CREATE INDEX events_occurred_idx ON public.events (occurred_at);

-- Views, function, procedure ---------------------------------------------

CREATE VIEW sales.order_totals AS
SELECT o.id AS order_id, c.name AS customer, o.status, o.total, count(i.product_id) AS item_count
FROM sales.orders o
JOIN public.customers c ON c.id = o.customer_id
LEFT JOIN sales.order_items i ON i.order_id = o.id
GROUP BY o.id, c.name, o.status, o.total;

CREATE VIEW inventory.low_stock AS
SELECT p.sku, p.name, w.name AS warehouse, s.on_hand
FROM inventory.stock s
JOIN inventory.products p ON p.id = s.product_id
JOIN inventory.warehouses w ON w.id = s.warehouse_id
WHERE s.on_hand < 10;

CREATE FUNCTION sales.customer_spend(p_customer uuid) RETURNS numeric
LANGUAGE sql STABLE AS $$
    SELECT coalesce(sum(total), 0) FROM sales.orders WHERE customer_id = p_customer AND status <> 'cancelled'
$$;

CREATE PROCEDURE inventory.restock(p_warehouse integer, p_amount integer)
LANGUAGE plpgsql AS $$
BEGIN
    UPDATE inventory.stock SET on_hand = on_hand + p_amount, updated_at = now() WHERE warehouse_id = p_warehouse;
END
$$;

ANALYZE;
