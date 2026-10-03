-- Whether "leave" counts towards the attendance percentage. Strict by default; confirm with the college.
INSERT INTO site_settings (key, value) VALUES ('attendance_leave_counts_as_present', 'false')
ON CONFLICT (key) DO NOTHING;
