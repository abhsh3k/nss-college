-- Where a section's photo sits relative to its heading and text.
--
-- Until now a section photo was always drawn above the heading. The editor now
-- offers a placement of its own — above the heading, below the section, beside
-- the text on the left or the right, or inline at a `[photo]` line the author
-- writes into the body — so an editor decides where the picture goes instead of
-- the template deciding for them.
--
-- The default is 'above', which is exactly what every existing section already
-- rendered, so adding the column changes no published page. The six display
-- columns added in `0011_section_display.sql` are unchanged; this is a seventh,
-- orthogonal choice (the horizontal alignment still applies when the photo is
-- above or below the text).

ALTER TABLE section_display_defaults
    ADD COLUMN photo_position TEXT NOT NULL DEFAULT 'above'
        CHECK (photo_position IN ('above', 'below', 'left', 'right', 'inline'));

ALTER TABLE home_sections
    ADD COLUMN photo_position TEXT NOT NULL DEFAULT 'above'
        CHECK (photo_position IN ('above', 'below', 'left', 'right', 'inline'));

ALTER TABLE page_sections
    ADD COLUMN photo_position TEXT NOT NULL DEFAULT 'above'
        CHECK (photo_position IN ('above', 'below', 'left', 'right', 'inline'));
