-- Starter content taken from the live nsscrky.ac.in homepage. Safe to edit from the admin dashboard later.

INSERT INTO departments (slug, name, summary, sort_order) VALUES ('computer-applications', 'Computer Applications', 'Offers the Bachelor of Computer Applications and hosts research in computer applications.', 1);
INSERT INTO departments (slug, name, summary, sort_order) VALUES ('electronics', 'Electronics', 'Offers B.Sc. and M.Sc. Electronics, with research in electronics.', 2);
INSERT INTO departments (slug, name, summary, sort_order) VALUES ('business-administration', 'Business Administration', 'Offers the Bachelor of Business Administration programme.', 3);
INSERT INTO departments (slug, name, summary, sort_order) VALUES ('commerce', 'Commerce', 'Offers B.Com (Co-operation) and B.Com with Computer Applications.', 4);
INSERT INTO programmes (department_id, slug, name, level, summary, sort_order) VALUES ((SELECT id FROM departments WHERE slug = 'computer-applications'), 'bca', 'Bachelor of Computer Applications', '4-year honours', 'Software and computing foundations, with room for independent thinking on real technical problems.', 1);
INSERT INTO programmes (department_id, slug, name, level, summary, sort_order) VALUES ((SELECT id FROM departments WHERE slug = 'electronics'), 'bsc-electronics', 'B.Sc. Electronics', '4-year honours', 'Digital and analog electronics, programming, network analysis and electromagnetics for the communication industry.', 2);
INSERT INTO programmes (department_id, slug, name, level, summary, sort_order) VALUES ((SELECT id FROM departments WHERE slug = 'business-administration'), 'bba', 'Bachelor of Business Administration', '4-year honours', 'Finance, economics, operations, marketing and accounting, with practical training.', 3);
INSERT INTO programmes (department_id, slug, name, level, summary, sort_order) VALUES ((SELECT id FROM departments WHERE slug = 'commerce'), 'bcom-cooperation', 'B.Com (Co-operation), Model I', '4-year honours', 'Commerce degree built on book-keeping and accountancy at Plus Two level.', 4);
INSERT INTO programmes (department_id, slug, name, level, summary, sort_order) VALUES ((SELECT id FROM departments WHERE slug = 'commerce'), 'bcom-ca', 'B.Com Computer Applications, Model II', '4-year honours', 'Accounting, finance and business management combined with programming and database skills.', 5);
INSERT INTO programmes (department_id, slug, name, level, summary, sort_order) VALUES ((SELECT id FROM departments WHERE slug = 'electronics'), 'msc-electronics', 'M.Sc. Electronics', '2-year postgraduate', 'Design and operation of electronic systems, with specialisation in computing and telecommunications.', 6);
INSERT INTO clubs (slug, short_name, full_name, summary, values_text, image_path, featured, sort_order) VALUES ('ncc', 'NCC', 'National Cadet Corps', 'A youth development movement that builds discipline, leadership and a sense of social responsibility through classroom learning and practical training.', 'Leadership, discipline and character, community service', 'https://nsscrky.ac.in/uploads/carousel/1779448436_41898e75d1e7aafa5607.png', true, 1);
INSERT INTO clubs (slug, short_name, full_name, summary, values_text, image_path, featured, sort_order) VALUES ('nss', 'NSS', 'National Service Scheme', 'A voluntary programme under the Ministry of Youth Affairs and Sports that gives students a platform for community service and personal growth.', 'Personality development, unity, service to the community', 'https://nsscrky.ac.in/uploads/carousel/1779450929_d2690612d1be7d04deba.png', true, 2);
INSERT INTO facilities (name, description, sort_order) VALUES ('Central library', 'Textbooks, journals and digital resources, with a reading hall.', 1);
INSERT INTO facilities (name, description, sort_order) VALUES ('Computer laboratories', 'High-speed internet and current software environments.', 2);
INSERT INTO facilities (name, description, sort_order) VALUES ('Electronics lab', 'Modern instruments for practical experiments.', 3);
INSERT INTO facilities (name, description, sort_order) VALUES ('Sports facilities', 'Multi-sport grounds, a gymnasium and athletics training areas.', 4);
INSERT INTO facilities (name, description, sort_order) VALUES ('Seminar hall', 'Air-conditioned, with AV facilities for seminars and workshops.', 5);
INSERT INTO facilities (name, description, sort_order) VALUES ('Student support cells', 'Anti-ragging, women''s cell, career guidance, counselling and grievance redressal.', 6);
INSERT INTO facilities (name, description, sort_order) VALUES ('Canteen and hostel', 'A hygienic canteen and hostel rooms for outstation students.', 7);
INSERT INTO milestones (when_label, description, sort_order) VALUES ('16 January 1995', 'Foundation stone of the permanent building laid by Sri. P.K. Narayana Panicker.', 1);
INSERT INTO milestones (when_label, description, sort_order) VALUES ('June 1995', 'The college opens in rented premises in Rajakumari town under the Nair Service Society.', 2);
INSERT INTO milestones (when_label, description, sort_order) VALUES ('7 March 2000', 'The hilltop campus near Kulapparachal is inaugurated.', 3);
INSERT INTO milestones (when_label, description, sort_order) VALUES ('2002 onwards', 'B.Com with Computer Applications is introduced and programmes continue to expand.', 4);
INSERT INTO rank_holders (name, rank_position, department_id, exam_year, photo_path, sort_order) VALUES ('Aparna Prasad', 3, (SELECT id FROM departments WHERE slug = 'computer-applications'), 2025, 'https://nsscrky.ac.in/uploads/toppers/1779001373_b57d21d0fa258fab1122.png', 1);
INSERT INTO rank_holders (name, rank_position, department_id, exam_year, photo_path, sort_order) VALUES ('Athulya Mohan', 6, (SELECT id FROM departments WHERE slug = 'commerce'), 2025, 'https://nsscrky.ac.in/uploads/toppers/1779000941_616a233022109612abc4.png', 2);
INSERT INTO rank_holders (name, rank_position, department_id, exam_year, photo_path, sort_order) VALUES ('Aparna Shaji', 7, (SELECT id FROM departments WHERE slug = 'commerce'), 2025, 'https://nsscrky.ac.in/uploads/toppers/1779001178_948e2dd34bff4e7a428f.png', 3);
INSERT INTO news (title, image_path, status, published_at) VALUES ('International Yoga Day', 'https://nsscrky.ac.in/uploads/news/1782723188_57b7230e7c1704b211c8.jpeg', 'published', '2026-06-29'::timestamptz);
INSERT INTO news (title, image_path, status, published_at) VALUES ('Rajakumari N.S.S. College declared a Green Campus', 'https://nsscrky.ac.in/uploads/news/1778998882_afbc4a587fef3a07fc5b.jpg', 'published', '2026-05-17 10:00'::timestamptz);
INSERT INTO news (title, image_path, status, published_at) VALUES ('Chief Minister''s mega quiz', 'https://nsscrky.ac.in/uploads/news/1778998807_6f56aa0d407f53479565.jpeg', 'published', '2026-05-17 09:00'::timestamptz);
INSERT INTO notices (title, category, attachment_path, audience, is_pinned, status, published_at) VALUES ('Admissions open for 2026–30: apply for undergraduate programmes', 'Admissions', '/admissions', 'public', true, 'published', now());
INSERT INTO notices (title, category, attachment_path, audience, is_pinned, status, published_at) VALUES ('Notice board update', 'Notice', NULL, 'public', false, 'published', now() - interval '30 days');
INSERT INTO pages (path, title, lede) VALUES ('/about', 'About the college', 'A Nair Service Society college serving the High Ranges of Idukki since 1995.');
INSERT INTO page_sections (page_id, heading, body, sort_order) VALUES ((SELECT id FROM pages WHERE path = '/about'), 'Management', 'The Nair Service Society was founded by Padma Bhushan Bharatha Kesari Mannathu Padmanabhan. The society opened this college to bring higher education to a region that had little of it.', 1);
INSERT INTO page_sections (page_id, heading, body, sort_order) VALUES ((SELECT id FROM pages WHERE path = '/about'), 'History', 'The foundation stone of the permanent building was laid on 16 January 1995 by Sri. P.K. Narayana Panicker. The college opened in June 1995 in rented premises in Rajakumari town.

The hilltop campus near Kulapparachal was inaugurated on 7 March 2000. B.Com with Computer Applications followed in 2002, and programmes have continued to grow since.', 2);
INSERT INTO page_sections (page_id, heading, body, sort_order) VALUES ((SELECT id FROM pages WHERE path = '/about'), 'Vision', 'To uplift the socio-economic backwardness of the High Ranges through job-oriented education in electronics, computer science, business administration and commerce.', 3);
INSERT INTO page_sections (page_id, heading, body, sort_order) VALUES ((SELECT id FROM pages WHERE path = '/about'), 'Mission', 'Help students of every programme excel in their professions.

Build community awareness through extension activities.

Nurture each student''s ability through curricular and co-curricular work.

Give practical expertise through well-equipped laboratories and in-house projects.

Turn laboratories into active research centres.', 4);
INSERT INTO pages (path, title, lede) VALUES ('/about/principal', 'Principal''s desk', 'A message from the Principal to students, parents and visitors.');
INSERT INTO pages (path, title, lede) VALUES ('/about/organogram', 'Organogram', 'How the college is organised, from management to departments and committees.');
INSERT INTO pages (path, title, lede) VALUES ('/about/council', 'College council', 'Members of the college council and its committees.');
INSERT INTO pages (path, title, lede) VALUES ('/about/staff', 'Teaching staff', 'Faculty by department, with qualifications and contact details.');
INSERT INTO pages (path, title, lede) VALUES ('/about/code-of-conduct', 'Code of conduct', 'The rules and expectations for students, faculty and staff.');
INSERT INTO pages (path, title, lede) VALUES ('/admissions', 'Admissions 2026–30', 'Admissions are open for undergraduate programmes.');
INSERT INTO page_sections (page_id, heading, body, sort_order) VALUES ((SELECT id FROM pages WHERE path = '/admissions'), 'Programmes', 'The college offers the Bachelor of Computer Applications, B.Sc. Electronics, Bachelor of Business Administration, B.Com (Co-operation) and B.Com with Computer Applications. See the programmes page for details of each.', 1);
INSERT INTO page_sections (page_id, heading, body, sort_order) VALUES ((SELECT id FROM pages WHERE path = '/admissions'), 'Talk to the admission office', 'Ask about eligibility, fees and scholarships before you apply. Call 04868-245370 or 04868-245515, or write to nssrajakumari@yahoo.com.', 2);
INSERT INTO pages (path, title, lede) VALUES ('/academics/syllabus', 'Syllabus', 'Course syllabi for every programme, as prescribed by Mahatma Gandhi University.');
INSERT INTO pages (path, title, lede) VALUES ('/academics/examinations', 'College examinations', 'Internal examination schedules, timetables and results.');
INSERT INTO pages (path, title, lede) VALUES ('/student-life/union', 'College union', 'The elected student body and its activities.');
INSERT INTO pages (path, title, lede) VALUES ('/student-life/clubs', 'Clubs and cells', 'Arts, sports, literary and community clubs open to every student.');
INSERT INTO pages (path, title, lede) VALUES ('/student-life/nss', 'National Service Scheme', 'A voluntary programme that gives students a platform for community service and personal growth.');
INSERT INTO page_sections (page_id, heading, body, sort_order) VALUES ((SELECT id FROM pages WHERE path = '/student-life/nss'), 'About the unit', 'The National Service Scheme runs under the Ministry of Youth Affairs and Sports. Its aims are personality development through community service, national unity, and service to the community.', 1);
INSERT INTO pages (path, title, lede) VALUES ('/student-life/ncc', 'National Cadet Corps', 'A youth development movement that builds discipline, leadership and a sense of social responsibility.');
INSERT INTO page_sections (page_id, heading, body, sort_order) VALUES ((SELECT id FROM pages WHERE path = '/student-life/ncc'), 'About the unit', 'Cadets learn through classroom instruction and practical training. The unit focuses on leadership, discipline and character, and community service.', 1);
INSERT INTO pages (path, title, lede) VALUES ('/student-life/scholarships', 'Scholarships', 'Scholarships and financial aid available to students, and how to apply.');
INSERT INTO pages (path, title, lede) VALUES ('/student-life/anti-ragging', 'Anti-ragging cell', 'How to report ragging and who will respond.');
INSERT INTO pages (path, title, lede) VALUES ('/alumni', 'Alumni', 'Stay connected with the college and with fellow graduates.');
INSERT INTO pages (path, title, lede) VALUES ('/iqac', 'IQAC', 'The Internal Quality Assurance Cell: reports, minutes and accreditation documents.');
INSERT INTO pages (path, title, lede) VALUES ('/placement', 'Placement', 'Campus recruitment, training and the placement cell.');
INSERT INTO pages (path, title, lede) VALUES ('/gallery', 'Gallery', 'Photographs from campus, events and celebrations.');
INSERT INTO pages (path, title, lede) VALUES ('/research', 'Research', 'Research programmes in electronics and computer applications.');
INSERT INTO pages (path, title, lede) VALUES ('/rti', 'Right to Information', 'How to file a request under the Right to Information Act.');
INSERT INTO pages (path, title, lede) VALUES ('/fees', 'Fees', 'Fee structure for each programme.');
INSERT INTO site_settings (key, value) VALUES ('contact_address', 'NSS College, Rajakumari P.O., Kulapparachal, Idukki, Kerala 685 619');
INSERT INTO site_settings (key, value) VALUES ('contact_phone_1', '04868-245370');
INSERT INTO site_settings (key, value) VALUES ('contact_phone_2', '04868-245515');
INSERT INTO site_settings (key, value) VALUES ('contact_email', 'nssrajakumari@yahoo.com');
