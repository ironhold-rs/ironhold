-- People who can log in. Emails are stored lowercased.
CREATE TABLE users (
    id INTEGER PRIMARY KEY,
    email TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
);
