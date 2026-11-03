CREATE TABLE posts (
    id        BIGSERIAL PRIMARY KEY,
    title     TEXT    NOT NULL,
    body      TEXT    NOT NULL,
    published BOOLEAN NOT NULL DEFAULT FALSE
);

INSERT INTO posts (title, body, published) VALUES
    ('Welcome to Oxide', 'This post was seeded by database/migrations.', TRUE),
    ('Drafting with Oxide', 'Drafts are posts with published set to false.', FALSE);
