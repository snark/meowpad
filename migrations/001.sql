CREATE TABLE IF NOT EXISTS link (
     id BLOB CHECK(length(id) = 16) PRIMARY KEY,
     url TEXT NOT NULL UNIQUE,
     title TEXT,
     description TEXT,
     is_primary BOOL DEFAULT TRUE,
     content TEXT,
     created_at DATETIME NOT NULL,
     modified_at DATETIME NOT NULL
);

CREATE TABLE IF NOT EXISTS note (
     id BLOB CHECK(length(id) = 16) PRIMARY KEY,
     content TEXT NOT NULL,
     title TEXT NOT NULL UNIQUE,
     link_id BLOB,
     created_at DATETIME NOT NULL,
     modified_at DATETIME NOT NULL,
     FOREIGN KEY(link_id) REFERENCES link(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS tag (
     id BLOB CHECK(length(id) = 16) PRIMARY KEY,
     name TEXT NOT NULL UNIQUE,
     slug TEXT NOT NULL,
     created_at DATETIME NOT NULL,
     modified_at DATETIME NOT NULL
);

CREATE TABLE IF NOT EXISTS item_tag (
     tag_id BLOB CHECK(length(tag_id) = 16) NOT NULL,
     note_id BLOB CHECK(length(note_id) = 16),
     link_id BLOB CHECK(length(link_id) = 16),
     FOREIGN KEY(tag_id) REFERENCES tag(id) ON DELETE CASCADE,
     FOREIGN KEY(link_id) REFERENCES link(id) ON DELETE CASCADE,
     FOREIGN KEY(note_id) REFERENCES note(id) ON DELETE CASCADE,
     UNIQUE(tag_id, link_id, note_id)
);

CREATE TABLE IF NOT EXISTS related_link (
     primary_link_id BLOB CHECK(length(primary_link_id) = 16) NOT NULL,
     related_link_id BLOB CHECK(length(related_link_id) = 16) NOT NULL,
     relationship TEXT,
     FOREIGN KEY(primary_link_id) REFERENCES link(id) ON DELETE CASCADE,
     FOREIGN KEY(related_link_id) REFERENCES link(id) ON DELETE CASCADE,
     UNIQUE(primary_link_id, related_link_id)
);


