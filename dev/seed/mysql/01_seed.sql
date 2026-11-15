-- Dev-only seed for manual exploration. Runs once, on first start of an empty volume.
-- Mapping: Postgres schemas public/sales/inventory become databases dbased_dev/dbased_sales/dbased_inventory.

CREATE DATABASE IF NOT EXISTS dbased_sales;
CREATE DATABASE IF NOT EXISTS dbased_inventory;
GRANT ALL PRIVILEGES ON dbased_sales.* TO 'dbased'@'%';
GRANT ALL PRIVILEGES ON dbased_inventory.* TO 'dbased'@'%';

SET SESSION cte_max_recursion_depth = 1000000;
SET SESSION sql_mode = 'STRICT_TRANS_TABLES,NO_ENGINE_SUBSTITUTION';

USE dbased_dev;

CREATE TABLE customers (
    id          char(36) NOT NULL PRIMARY KEY,
    name        varchar(100) NOT NULL,
    email       varchar(150) NOT NULL UNIQUE,
    tags        set('vip', 'newsletter', 'beta', 'staff') NOT NULL DEFAULT '',
    profile     json NOT NULL,
    flags       bit(8) NOT NULL DEFAULT b'00000001',
    created_at  datetime(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3)
) COMMENT 'Customers with uuid-like key, set, json and bit columns.';

CREATE TABLE app_settings (
    `key`  varchar(64) NOT NULL PRIMARY KEY,
    value  json NOT NULL
);

-- No primary key and no unique index: exercises the read-only-by-default tier.
-- The datetime column accepts zero dates when sql_mode allows them.
CREATE TABLE audit_log (
    happened_at  datetime NOT NULL DEFAULT CURRENT_TIMESTAMP,
    actor        varchar(50),
    action       varchar(50) NOT NULL,
    details      json
);

CREATE TABLE legacy_dates (
    id        int NOT NULL PRIMARY KEY,
    created   datetime NULL,
    expires   date NULL
);

CREATE TABLE events (
    id           bigint NOT NULL AUTO_INCREMENT PRIMARY KEY,
    customer_id  char(36) NULL,
    kind         enum('click', 'view', 'purchase', 'refund') NOT NULL,
    amount       decimal(12, 2) NOT NULL,
    payload      json NOT NULL,
    occurred_at  datetime(3) NOT NULL
);

USE dbased_inventory;

CREATE TABLE products (
    id      int NOT NULL AUTO_INCREMENT PRIMARY KEY,
    sku     varchar(20) NOT NULL UNIQUE,
    name    varchar(100) NOT NULL,
    price   decimal(12, 2) NOT NULL,
    attrs   json NOT NULL,
    active  tinyint(1) NOT NULL DEFAULT 1
);

CREATE TABLE warehouses (
    id    int NOT NULL AUTO_INCREMENT PRIMARY KEY,
    name  varchar(100) NOT NULL,
    city  varchar(100)
);

-- Composite primary key.
CREATE TABLE stock (
    warehouse_id  int NOT NULL,
    product_id    int NOT NULL,
    on_hand       int NOT NULL DEFAULT 0,
    updated_at    datetime(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    PRIMARY KEY (warehouse_id, product_id),
    KEY stock_product_idx (product_id),
    CONSTRAINT stock_warehouse_fk FOREIGN KEY (warehouse_id) REFERENCES warehouses (id),
    CONSTRAINT stock_product_fk FOREIGN KEY (product_id) REFERENCES products (id)
);

CREATE TABLE suppliers (
    id       int NOT NULL AUTO_INCREMENT PRIMARY KEY,
    name     varchar(100) NOT NULL,
    contact  json
);

USE dbased_sales;

CREATE TABLE orders (
    id           bigint NOT NULL AUTO_INCREMENT PRIMARY KEY,
    customer_id  char(36) NOT NULL,
    status       enum('pending', 'paid', 'shipped', 'cancelled') NOT NULL DEFAULT 'pending',
    total        decimal(12, 2) NOT NULL DEFAULT 0,
    placed_at    datetime(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    notes        text,
    KEY orders_customer_idx (customer_id),
    KEY orders_status_placed_idx (status, placed_at),
    CONSTRAINT orders_customer_fk FOREIGN KEY (customer_id) REFERENCES dbased_dev.customers (id)
);

-- Composite primary key.
CREATE TABLE order_items (
    order_id    bigint NOT NULL,
    product_id  int NOT NULL,
    quantity    int NOT NULL,
    unit_price  decimal(12, 2) NOT NULL,
    PRIMARY KEY (order_id, product_id),
    KEY order_items_product_idx (product_id),
    CONSTRAINT order_items_order_fk FOREIGN KEY (order_id) REFERENCES orders (id) ON DELETE CASCADE,
    CONSTRAINT order_items_product_fk FOREIGN KEY (product_id) REFERENCES dbased_inventory.products (id),
    CONSTRAINT order_items_quantity_chk CHECK (quantity > 0)
);

-- Data -----------------------------------------------------------------

USE dbased_dev;

INSERT INTO customers (id, name, email, tags, profile, flags, created_at)
WITH RECURSIVE seq (n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM seq WHERE n < 200)
SELECT UUID(), CONCAT('Customer ', n), CONCAT('customer', n, '@example.test'),
       ELT(1 + n % 4, 'vip', 'newsletter', 'beta,newsletter', 'staff'),
       JSON_OBJECT('age', 18 + n % 60, 'vip', n % 10 = 0, 'address', JSON_OBJECT('city', CONCAT('City ', n % 20))),
       CAST(n % 256 AS UNSIGNED),
       NOW(3) - INTERVAL n HOUR
FROM seq;

INSERT INTO app_settings VALUES ('theme', '{"mode": "dark"}'), ('limits', '{"rows": 1000, "timeout_s": 30}');

INSERT INTO audit_log (actor, action, details)
WITH RECURSIVE seq (n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM seq WHERE n < 50)
SELECT CONCAT('user', n % 7), ELT(1 + n % 4, 'login', 'logout', 'update', 'delete'), JSON_OBJECT('n', n)
FROM seq;

INSERT INTO legacy_dates VALUES (1, '2024-01-01 10:00:00', '2024-12-31'), (2, NULL, NULL);
-- @NOTE: a literal zero date needs a relaxed sql_mode for this session only.
SET SESSION sql_mode = 'NO_ENGINE_SUBSTITUTION';
INSERT INTO legacy_dates VALUES (3, '0000-00-00 00:00:00', '0000-00-00');
SET SESSION sql_mode = 'STRICT_TRANS_TABLES,NO_ENGINE_SUBSTITUTION';

INSERT INTO dbased_inventory.products (sku, name, price, attrs)
WITH RECURSIVE seq (n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM seq WHERE n < 100)
SELECT CONCAT('SKU-', LPAD(n, 5, '0')), CONCAT('Product ', n), ROUND(RAND() * 500 + 1, 2),
       JSON_OBJECT('color', ELT(1 + n % 3, 'red', 'green', 'blue'), 'weight_kg', n % 9 + 0.5)
FROM seq;

INSERT INTO dbased_inventory.warehouses (name, city) VALUES ('Main', 'Sao Paulo'), ('North', 'Recife'), ('South', 'Porto Alegre');

INSERT INTO dbased_inventory.stock (warehouse_id, product_id, on_hand)
SELECT w.id, p.id, (w.id * p.id) % 200
FROM dbased_inventory.warehouses w CROSS JOIN dbased_inventory.products p;

INSERT INTO dbased_inventory.suppliers (name, contact)
WITH RECURSIVE seq (n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM seq WHERE n < 15)
SELECT CONCAT('Supplier ', n), JSON_OBJECT('email', CONCAT('sales@supplier', n, '.test')) FROM seq;

INSERT INTO dbased_sales.orders (customer_id, status, total, placed_at)
WITH RECURSIVE seq (n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM seq WHERE n < 2000)
SELECT (SELECT c.id FROM (SELECT id, ROW_NUMBER() OVER (ORDER BY id) AS rn FROM dbased_dev.customers) c WHERE c.rn = 1 + n % 200),
       ELT(1 + n % 4, 'pending', 'paid', 'shipped', 'cancelled'), 0, NOW(3) - INTERVAL n HOUR
FROM seq;

INSERT INTO dbased_sales.order_items (order_id, product_id, quantity, unit_price)
SELECT o.id, p.id, 1 + (o.id + p.id) % 5, p.price
FROM dbased_sales.orders o
JOIN dbased_inventory.products p ON p.id IN (1 + o.id % 100, 1 + (o.id * 7) % 100, 1 + (o.id * 13) % 100);

UPDATE dbased_sales.orders o
JOIN (SELECT order_id, SUM(quantity * unit_price) AS total FROM dbased_sales.order_items GROUP BY order_id) s
  ON s.order_id = o.id
SET o.total = s.total;

-- Large table for scrolling and keyset pagination (~1,000,000 rows).
INSERT INTO events (kind, amount, payload, occurred_at)
WITH RECURSIVE seq (n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM seq WHERE n < 1000000)
SELECT ELT(1 + n % 4, 'click', 'view', 'purchase', 'refund'), ROUND((n % 10000) / 100, 2),
       JSON_OBJECT('seq', n, 'ok', n % 3 <> 0),
       TIMESTAMP('2024-01-01 00:00:00') + INTERVAL n SECOND
FROM seq;
CREATE INDEX events_occurred_idx ON events (occurred_at);

-- Views, function, procedure ---------------------------------------------

CREATE VIEW dbased_sales.order_totals AS
SELECT o.id AS order_id, c.name AS customer, o.status, o.total, COUNT(i.product_id) AS item_count
FROM dbased_sales.orders o
JOIN dbased_dev.customers c ON c.id = o.customer_id
LEFT JOIN dbased_sales.order_items i ON i.order_id = o.id
GROUP BY o.id, c.name, o.status, o.total;

CREATE VIEW dbased_inventory.low_stock AS
SELECT p.sku, p.name, w.name AS warehouse, s.on_hand
FROM dbased_inventory.stock s
JOIN dbased_inventory.products p ON p.id = s.product_id
JOIN dbased_inventory.warehouses w ON w.id = s.warehouse_id
WHERE s.on_hand < 10;

DELIMITER $$

CREATE FUNCTION dbased_sales.customer_spend(p_customer char(36)) RETURNS decimal(12, 2)
READS SQL DATA
BEGIN
    RETURN (SELECT COALESCE(SUM(total), 0) FROM dbased_sales.orders WHERE customer_id = p_customer AND status <> 'cancelled');
END$$

CREATE PROCEDURE dbased_inventory.restock(IN p_warehouse int, IN p_amount int)
BEGIN
    UPDATE dbased_inventory.stock SET on_hand = on_hand + p_amount, updated_at = NOW(3) WHERE warehouse_id = p_warehouse;
END$$

DELIMITER ;

ANALYZE TABLE dbased_dev.events;
