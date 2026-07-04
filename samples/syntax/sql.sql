/*
 * Syntax gallery sample — SQL.
 *
 * This block comment is prose: it explains the file's purpose in full
 * sentences, so it should render prominent rather than fading like the
 * commented-out code below.
 */

-- retries = 3;
-- select * from accounts;

CREATE TABLE accounts (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    verbose BOOLEAN DEFAULT FALSE,
    retries INTEGER DEFAULT 5,
    tag CHAR(1) DEFAULT 'c'
);

CREATE VIEW active_accounts AS
    SELECT id, name FROM accounts WHERE verbose = TRUE;

CREATE INDEX IF NOT EXISTS idx_accounts_name ON accounts (name);

CREATE FUNCTION greet(host TEXT) RETURNS TEXT AS $$
    SELECT 'hello, ' || host;
$$ LANGUAGE sql;

INSERT INTO accounts (id, name, verbose, retries, tag)
VALUES (1, 'hello, awl', FALSE, 5, 'c');

SELECT name, retries
FROM accounts
WHERE verbose = FALSE AND tag = 'c' AND name IS NOT NULL
ORDER BY retries DESC;

CREATE TRIGGER trg_accounts_touch
    AFTER UPDATE ON accounts
    FOR EACH ROW
    EXECUTE FUNCTION greet(NULL);

UPDATE accounts SET retries = retries + 1 WHERE verbose = TRUE;
