-- How each block of the public site is presented.
--
-- Every section — a home page block, a section of an informational page, and
-- the rank holders list — is shown as cards or as plain paragraphs, over one to
-- four columns, with the photo and the text aligned left, centre or right, and
-- the photo cropped as a circle, a rounded rectangle or a square at one of three
-- sizes. Storing those six choices beside the copy means the IT admin can restyle
-- any block from the settings screen instead of a code change.
--
-- `section_display_defaults` holds the single row that new sections start from,
-- so the whole site can be given one look and later changed in one place.
--
-- The defaults below deliberately match what the templates hardcoded until now
-- (cards, three columns, everything left, a round photo), so adding these columns
-- changes no page that is already published.

CREATE TABLE section_display_defaults (
    id          INT         PRIMARY KEY DEFAULT 1 CHECK (id = 1),
    layout      TEXT        NOT NULL DEFAULT 'cards'  CHECK (layout IN ('cards', 'paragraphs')),
    grid_columns INT        NOT NULL DEFAULT 3        CHECK (grid_columns BETWEEN 1 AND 4),
    image_align TEXT        NOT NULL DEFAULT 'left'   CHECK (image_align IN ('left', 'center', 'right')),
    text_align  TEXT        NOT NULL DEFAULT 'left'   CHECK (text_align IN ('left', 'center', 'right')),
    photo_shape TEXT        NOT NULL DEFAULT 'circle' CHECK (photo_shape IN ('circle', 'rounded', 'square')),
    photo_size  TEXT        NOT NULL DEFAULT 'md'     CHECK (photo_size IN ('sm', 'md', 'lg')),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

INSERT INTO section_display_defaults (id) VALUES (1);

CREATE TRIGGER section_display_defaults_set_updated_at
    BEFORE UPDATE ON section_display_defaults
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- The six choices, plus an optional photo for the block as a whole.
--
-- `photo_path` points into the uploads directory and is recorded in `uploads`,
-- the same way a notice attachment or a rank holder's own photo already is.
ALTER TABLE home_sections
    ADD COLUMN layout       TEXT NOT NULL DEFAULT 'cards',
    ADD COLUMN grid_columns INT  NOT NULL DEFAULT 3,
    ADD COLUMN image_align  TEXT NOT NULL DEFAULT 'left',
    ADD COLUMN text_align   TEXT NOT NULL DEFAULT 'left',
    ADD COLUMN photo_shape  TEXT NOT NULL DEFAULT 'circle',
    ADD COLUMN photo_size   TEXT NOT NULL DEFAULT 'md',
    ADD COLUMN photo_path   TEXT;

ALTER TABLE page_sections
    ADD COLUMN layout       TEXT NOT NULL DEFAULT 'cards',
    ADD COLUMN grid_columns INT  NOT NULL DEFAULT 3,
    ADD COLUMN image_align  TEXT NOT NULL DEFAULT 'left',
    ADD COLUMN text_align   TEXT NOT NULL DEFAULT 'left',
    ADD COLUMN photo_shape  TEXT NOT NULL DEFAULT 'circle',
    ADD COLUMN photo_size   TEXT NOT NULL DEFAULT 'md',
    ADD COLUMN photo_path   TEXT;

-- The rank holders block on the home page is the one the reader sees as a row of
-- cards; it keeps that look. Every other home block reads better as prose, so the
-- seeded rows that hold sentences rather than a list of people become paragraphs.
UPDATE home_sections SET layout = 'paragraphs' WHERE section_key IN (
    'hero', 'programmes', 'admissions', 'history', 'vision', 'mission'
);