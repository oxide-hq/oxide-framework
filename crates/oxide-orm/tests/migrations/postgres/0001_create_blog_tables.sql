CREATE TABLE posts (
    id        BIGSERIAL PRIMARY KEY,
    author_id BIGINT  NOT NULL,
    title     TEXT    NOT NULL,
    body      TEXT    NOT NULL,
    published BOOLEAN NOT NULL DEFAULT FALSE
);

CREATE TABLE comments (
    id         BIGSERIAL PRIMARY KEY,
    post_id    BIGINT NOT NULL REFERENCES posts(id) ON DELETE CASCADE,
    body       TEXT   NOT NULL,
    rating     BIGINT,
    created_on DATE   NOT NULL
);
