CREATE VIRTUAL TABLE link_content
USING FTS5(link_id, content);

INSERT INTO link_content(link_id, content)
SELECT id, content FROM link;

ALTER TABLE link DROP COLUMN content;
