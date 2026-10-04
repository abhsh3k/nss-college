-- College facts and homepage copy move out of the templates and into the database.
--
-- `site_settings` holds single values (names, phone, legal line, error copy).
-- `home_sections` holds the homepage blocks, each addressed by a stable
-- `section_key` so the template never has to hardcode a string.

CREATE TABLE home_sections (
    section_key TEXT        PRIMARY KEY,
    heading     TEXT        NOT NULL DEFAULT '',
    body        TEXT        NOT NULL DEFAULT '',
    sort_order  INT         NOT NULL DEFAULT 0,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TRIGGER home_sections_set_updated_at
    BEFORE UPDATE ON home_sections
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

INSERT INTO site_settings (key, value) VALUES
    ('site_name', 'NSS College Rajakumari'),
    ('site_tagline', 'Affiliated to Mahatma Gandhi University'),
    ('site_header_note', 'Aided college, NAAC B+ (2.70)'),
    ('site_meta_description', 'NSS College Rajakumari, affiliated to Mahatma Gandhi University, offers BCA, B.Sc. Electronics, BBA and B.Com programmes in the High Ranges of Idukki, Kerala.'),
    ('site_footer_tagline', 'A college of the Nair Service Society, serving the High Ranges of Idukki since 1995.'),
    ('site_footer_legal', '© {year} {site_name}. Affiliated to Mahatma Gandhi University, Kottayam.'),
    ('site_map_embed_url', 'https://www.google.com/maps/embed?pb=!1m18!1m12!1m3!1d3929.654177138351!2d77.16909861428178!3d9.96270327641009!2m3!1f0!2f0!3f0!3m2!1i1024!2i768!4f13.1!3m3!1m2!1s0x3b07a0f52b1a501d%3A0x5e52b98bfcc0d3e7!2sNSS+College!5e0!3m2!1sen!2sin!4v1544351228773'),
    ('site_map_title', 'Map showing NSS College Rajakumari'),
    ('site_admissions_cta', 'Apply for 2026–30'),
    ('site_research_note', 'Research programmes are also offered in electronics and in computer applications.'),
    ('site_university_short', 'MG University'),
    ('contact_title', 'Contact and location'),
    ('contact_lede', 'The campus is on a hilltop near Kulapparachal, 2 km east of Rajakumari town, in Idukki district.'),
    ('page_placeholder_heading', 'We''re still preparing this page'),
    ('page_placeholder_body', 'Until it is ready, the college office can answer your questions. Call {phone} or find other ways to reach us.'),
    ('login_description', 'Sign in to the {site_name} Student Hub, teacher tools or administration.'),
    ('error_404_heading', 'This page isn''t here'),
    ('error_404_body', 'The address may have changed while we rebuilt the site. Try the home page, or find a programme or notice from the menu.'),
    ('error_403_heading', 'You can''t open this page'),
    ('error_403_body', 'Your account doesn''t have access to this area, or the form you sent had expired. Go back and try again, or sign in with a different account.'),
    ('error_400_heading', 'That didn''t look right'),
    ('error_400_body', 'The form you sent wasn''t valid, so nothing was saved. Go back, fix the highlighted problem and try again.'),
    ('error_500_heading', 'We couldn''t load this page'),
    ('error_500_body', 'The problem is on our side. Try again in a minute, or call the college office on {phone}.');

-- Title and ledes for the list pages, editable like any other informational page.
INSERT INTO pages (path, title, lede) VALUES
    ('/academics', 'Degree programmes', 'All programmes are affiliated to Mahatma Gandhi University, Kottayam. Undergraduate programmes follow the four-year honours structure.'),
    ('/departments', 'Departments', 'Four teaching departments, with research in electronics and computer applications.'),
    ('/news', 'News and events', 'Reports from campus: celebrations, achievements and programmes.'),
    ('/notices', 'Notices', 'Official notices for students, parents and applicants. This list updates while you read.'),
    ('/academics/rank-holders', 'University rank holders', 'Students who placed among the top ranks in Mahatma Gandhi University examinations.');

INSERT INTO home_sections (section_key, heading, body, sort_order) VALUES
    ('hero',
     'Learning on the hill above Rajakumari',
     'Five undergraduate programmes, a postgraduate programme in electronics and research in computer applications, on a hilltop campus near Kulapparachal.',
     1),
    ('stat_founded', 'Founded', 'June 1995, by the Nair Service Society', 2),
    ('stat_accreditation', 'Accreditation', 'NAAC B+ grade (2.70)', 3),
    ('stat_affiliation', 'Affiliation', 'Mahatma Gandhi University, Kottayam', 4),
    ('latest', 'What''s happening on campus', '', 5),
    ('programmes', 'Degree programmes',
     'All programmes are affiliated to Mahatma Gandhi University, Kottayam, and follow the four-year honours structure at undergraduate level.',
     6),
    ('admissions', 'Admissions are open for 2026–30',
     'Ask the admission office about eligibility, fees and scholarships before you apply.',
     7),
    ('rank_holders', 'University rank holders', '', 8),
    ('units', 'Service and discipline beyond the classroom', '', 9),
    ('history', 'A college for the High Ranges',
     'The Nair Service Society, founded by Padma Bhushan Bharatha Kesari Mannathu Padmanabhan, opened this college to bring higher education to a region that had little of it. The campus looks out over the hills and valleys of Kerala''s High Ranges.',
     10),
    ('vision', 'Our vision',
     'To uplift the socio-economic backwardness of the High Ranges through job-oriented education in electronics, computer science, business administration and commerce.',
     11),
    ('mission', 'Our mission',
     'Help students of every programme excel in their professions.

Build community awareness through extension activities.

Nurture each student''s ability through curricular and co-curricular work.

Give practical expertise through well-equipped laboratories and in-house projects.

Turn laboratories into active research centres.',
     12),
    ('milestones', 'Milestones', '', 13),
    ('facilities', 'Campus facilities', '', 14),
    ('contact', 'Visit or write to us', '', 15);