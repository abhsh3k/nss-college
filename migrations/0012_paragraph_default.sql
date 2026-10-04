-- Prose is the default layout, not cards.
--
-- 0011 added the display columns with `cards` as the column default, which was
-- wrong for the sections already on the site: an informational page is a page of
-- sentences, and drawing each of its sections as a three-card grid changed every
-- published page the moment the migration ran.
--
-- This corrects it in the other direction, so a page that looked right before
-- still looks right now:
--
--   * every existing page section goes back to `paragraphs`;
--   * the site-wide default becomes `paragraphs`, so a section added from now on
--     opens as prose too;
--   * the rank holders block stays on `cards`, because that block really is a
--     list of people and the card grid is what suits it.
--
-- Cards stay fully available: an admin picks them per section, on any page.

UPDATE page_sections SET layout = 'paragraphs';

UPDATE section_display_defaults SET layout = 'paragraphs' WHERE id = 1;