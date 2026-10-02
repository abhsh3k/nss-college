DO $$
DECLARE
    t text;
BEGIN
    FOREACH t IN ARRAY ARRAY[
        'users', 'departments', 'programmes', 'faculty', 'students', 'courses',
        'timetable_entries', 'attendance', 'exams', 'marks', 'study_materials',
        'notices', 'news', 'events', 'documents', 'rank_holders', 'clubs',
        'facilities', 'milestones', 'pages', 'page_sections', 'site_settings'
    ]
    LOOP
        EXECUTE format(
            'CREATE TRIGGER %I BEFORE UPDATE ON %I FOR EACH ROW EXECUTE FUNCTION set_updated_at()',
            t || '_set_updated_at', t
        );
    END LOOP;
END $$;
