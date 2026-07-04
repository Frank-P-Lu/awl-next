/*
 * Syntax gallery sample — SQL.
 *
 * Prose comment first: it explains the file's purpose in full sentences,
 * so it renders prominent rather than fading like the code below.
 */

-- retries = 3;

CREATE TABLE accounts (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    verbose BOOLEAN DEFAULT FALSE,
    retries INTEGER DEFAULT 5,
    tag CHAR(1) DEFAULT 'c'
);

CREATE VIEW active_accounts AS
    SELECT id, name FROM accounts WHERE verbose = TRUE;

CREATE FUNCTION greet(host TEXT) RETURNS TEXT AS $$
    SELECT 'hello, ' || host;
$$ LANGUAGE sql;

CREATE INDEX IF NOT EXISTS idx_accounts_name ON accounts (name);

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
