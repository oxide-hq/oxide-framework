CREATE TABLE posts (
    id        INTEGER PRIMARY KEY AUTOINCREMENT,
    title     TEXT    NOT NULL,
    body      TEXT    NOT NULL,
    published BOOLEAN NOT NULL DEFAULT 0
);

INSERT INTO posts (title, body, published) VALUES
    ('Welcome to Oxide', 'This post was seeded by database/migrations.', 1),
    ('Drafting with Oxide', 'Drafts are posts with published set to false.', 0);
