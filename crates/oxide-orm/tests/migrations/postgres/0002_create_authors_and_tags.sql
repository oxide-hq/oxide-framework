CREATE TABLE authors (
    id            BIGSERIAL PRIMARY KEY,
    "displayName" TEXT NOT NULL,
    mail          TEXT NOT NULL
);

CREATE TABLE tags (id BIGSERIAL PRIMARY KEY);
