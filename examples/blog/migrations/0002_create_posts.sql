-- Blog posts. Deleting a user deletes their posts.
CREATE TABLE posts (
    id INTEGER PRIMARY KEY,
    author_id INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    body TEXT NOT NULL,
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    updated_at INTEGER NOT NULL DEFAULT (unixepoch())
);

CREATE INDEX posts_created_at ON posts (created_at);
