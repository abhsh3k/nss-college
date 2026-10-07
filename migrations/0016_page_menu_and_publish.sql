-- Three gaps in the Pages section of the admin dashboard, fixed in one place:
--
--  1. The public header menu was hardcoded in `templates/partials/header.html`,
--     so a page created in the admin never appeared in the menu until a
--     developer edited the template. `pages` now records where (and whether)
--     each page sits in the menu, and the header renders from those rows.
--  2. A section could not be taken offline without deleting it. `published`
--     lets an admin keep a section (and its photo) but stop showing it.
--  3. An uploaded photo had nowhere to record a caption.
--
-- The seeds below copy the exact labels and order of the menu that was
-- hardcoded until now, so running this migration changes nothing a visitor
-- sees; from here on the admin edits the menu like any other page field.

ALTER TABLE pages
    ADD COLUMN nav_group TEXT        NOT NULL DEFAULT 'none',
    ADD COLUMN nav_label TEXT        NOT NULL DEFAULT '',
    ADD COLUMN nav_sort  INT         NOT NULL DEFAULT 0;

ALTER TABLE pages
    ADD CONSTRAINT pages_nav_group_check
    CHECK (nav_group IN ('none', 'utility', 'top', 'about', 'academics', 'student-life'));

ALTER TABLE page_sections
    ADD COLUMN published     BOOLEAN NOT NULL DEFAULT true,
    ADD COLUMN photo_caption TEXT    NOT NULL DEFAULT '';

ALTER TABLE home_sections
    ADD COLUMN published     BOOLEAN NOT NULL DEFAULT true,
    ADD COLUMN photo_caption TEXT    NOT NULL DEFAULT '';

-- The dark strip above the header: IQAC, Placement, Gallery, RTI, Fees.
UPDATE pages SET nav_group = 'utility', nav_label = 'IQAC',                 nav_sort = 1 WHERE path = '/iqac';
UPDATE pages SET nav_group = 'utility', nav_label = 'Placement',            nav_sort = 2 WHERE path = '/placement';
UPDATE pages SET nav_group = 'utility', nav_label = 'Gallery',              nav_sort = 3 WHERE path = '/gallery';
UPDATE pages SET nav_group = 'utility', nav_label = 'RTI',                  nav_sort = 4 WHERE path = '/rti';
UPDATE pages SET nav_group = 'utility', nav_label = 'Fees',                 nav_sort = 5 WHERE path = '/fees';

-- The "About" menu. The label differs from the page title on a few rows, so it
-- is written out here rather than left to fall back to the title.
UPDATE pages SET nav_group = 'about', nav_label = 'Our management',         nav_sort = 1 WHERE path = '/about';
UPDATE pages SET nav_group = 'about', nav_label = 'Principal''s desk',      nav_sort = 2 WHERE path = '/about/principal';
UPDATE pages SET nav_group = 'about', nav_label = 'Organogram',             nav_sort = 3 WHERE path = '/about/organogram';
UPDATE pages SET nav_group = 'about', nav_label = 'College council',        nav_sort = 4 WHERE path = '/about/council';
UPDATE pages SET nav_group = 'about', nav_label = 'Teaching staff',         nav_sort = 5 WHERE path = '/about/staff';
UPDATE pages SET nav_group = 'about', nav_label = 'Code of conduct',        nav_sort = 6 WHERE path = '/about/code-of-conduct';

-- The "Academics" menu.
UPDATE pages SET nav_group = 'academics', nav_label = 'All programmes',     nav_sort = 1 WHERE path = '/academics';
UPDATE pages SET nav_group = 'academics', nav_label = 'Syllabus',           nav_sort = 2 WHERE path = '/academics/syllabus';
UPDATE pages SET nav_group = 'academics', nav_label = 'College examinations', nav_sort = 3 WHERE path = '/academics/examinations';
UPDATE pages SET nav_group = 'academics', nav_label = 'University rank holders', nav_sort = 4 WHERE path = '/academics/rank-holders';
UPDATE pages SET nav_group = 'academics', nav_label = 'Departments',        nav_sort = 5 WHERE path = '/departments';

-- The "Student life" menu.
UPDATE pages SET nav_group = 'student-life', nav_label = 'College union',   nav_sort = 1 WHERE path = '/student-life/union';
UPDATE pages SET nav_group = 'student-life', nav_label = 'Clubs and cells', nav_sort = 2 WHERE path = '/student-life/clubs';
UPDATE pages SET nav_group = 'student-life', nav_label = 'NSS',             nav_sort = 3 WHERE path = '/student-life/nss';
UPDATE pages SET nav_group = 'student-life', nav_label = 'NCC',             nav_sort = 4 WHERE path = '/student-life/ncc';
UPDATE pages SET nav_group = 'student-life', nav_label = 'Scholarships',    nav_sort = 5 WHERE path = '/student-life/scholarships';
UPDATE pages SET nav_group = 'student-life', nav_label = 'Anti-ragging cell', nav_sort = 6 WHERE path = '/student-life/anti-ragging';

-- Top-level links after the three menus. "Contact" is a page of its own in the
-- router rather than a row here, so the header still draws it statically after
-- whatever this group holds.
UPDATE pages SET nav_group = 'top', nav_label = 'Alumni', nav_sort = 1 WHERE path = '/alumni';
UPDATE pages SET nav_group = 'top', nav_label = 'News',   nav_sort = 2 WHERE path = '/news';
