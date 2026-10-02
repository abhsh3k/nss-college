//! Static informational pages. One entry = one route, one nav target, one sitemap line.
//! Layer 3 moves these into a `pages` table editable from the admin dashboard;
//! the route registration below stays the same.

pub struct Section {
    pub heading: &'static str,
    pub paras: &'static [&'static str],
}

pub struct PageDef {
    pub path: &'static str,
    pub title: &'static str,
    pub lede: &'static str,
    pub sections: &'static [Section],
}

const NONE: &[Section] = &[];

pub static PAGES: &[PageDef] = &[
    PageDef {
        path: "/about",
        title: "About the college",
        lede: "A Nair Service Society college serving the High Ranges of Idukki since 1995.",
        sections: &[
            Section {
                heading: "Management",
                paras: &["The Nair Service Society was founded by Padma Bhushan Bharatha Kesari Mannathu Padmanabhan. The society opened this college to bring higher education to a region that had little of it."],
            },
            Section {
                heading: "History",
                paras: &[
                    "The foundation stone of the permanent building was laid on 16 January 1995 by Sri. P.K. Narayana Panicker. The college opened in June 1995 in rented premises in Rajakumari town.",
                    "The hilltop campus near Kulapparachal was inaugurated on 7 March 2000. B.Com with Computer Applications followed in 2002, and programmes have continued to grow since.",
                ],
            },
            Section {
                heading: "Vision",
                paras: &["To uplift the socio-economic backwardness of the High Ranges through job-oriented education in electronics, computer science, business administration and commerce."],
            },
            Section {
                heading: "Mission",
                paras: &[
                    "Help students of every programme excel in their professions.",
                    "Build community awareness through extension activities.",
                    "Nurture each student's ability through curricular and co-curricular work.",
                    "Give practical expertise through well-equipped laboratories and in-house projects.",
                    "Turn laboratories into active research centres.",
                ],
            },
        ],
    },
    PageDef { path: "/about/principal", title: "Principal's desk", lede: "A message from the Principal to students, parents and visitors.", sections: NONE },
    PageDef { path: "/about/organogram", title: "Organogram", lede: "How the college is organised, from management to departments and committees.", sections: NONE },
    PageDef { path: "/about/council", title: "College council", lede: "Members of the college council and its committees.", sections: NONE },
    PageDef { path: "/about/staff", title: "Teaching staff", lede: "Faculty by department, with qualifications and contact details.", sections: NONE },
    PageDef { path: "/about/code-of-conduct", title: "Code of conduct", lede: "The rules and expectations for students, faculty and staff.", sections: NONE },
    PageDef {
        path: "/admissions",
        title: "Admissions 2026–30",
        lede: "Admissions are open for undergraduate programmes.",
        sections: &[
            Section {
                heading: "Programmes",
                paras: &["The college offers the Bachelor of Computer Applications, B.Sc. Electronics, Bachelor of Business Administration, B.Com (Co-operation) and B.Com with Computer Applications. See the programmes page for details of each."],
            },
            Section {
                heading: "Talk to the admission office",
                paras: &["Ask about eligibility, fees and scholarships before you apply. Call 04868-245370 or 04868-245515, or write to nssrajakumari@yahoo.com."],
            },
        ],
    },
    PageDef { path: "/academics/syllabus", title: "Syllabus", lede: "Course syllabi for every programme, as prescribed by Mahatma Gandhi University.", sections: NONE },
    PageDef { path: "/academics/examinations", title: "College examinations", lede: "Internal examination schedules, timetables and results.", sections: NONE },
    PageDef { path: "/student-life/union", title: "College union", lede: "The elected student body and its activities.", sections: NONE },
    PageDef { path: "/student-life/clubs", title: "Clubs and cells", lede: "Arts, sports, literary and community clubs open to every student.", sections: NONE },
    PageDef {
        path: "/student-life/nss",
        title: "National Service Scheme",
        lede: "A voluntary programme that gives students a platform for community service and personal growth.",
        sections: &[Section {
            heading: "About the unit",
            paras: &["The National Service Scheme runs under the Ministry of Youth Affairs and Sports. Its aims are personality development through community service, national unity, and service to the community."],
        }],
    },
    PageDef {
        path: "/student-life/ncc",
        title: "National Cadet Corps",
        lede: "A youth development movement that builds discipline, leadership and a sense of social responsibility.",
        sections: &[Section {
            heading: "About the unit",
            paras: &["Cadets learn through classroom instruction and practical training. The unit focuses on leadership, discipline and character, and community service."],
        }],
    },
    PageDef { path: "/student-life/scholarships", title: "Scholarships", lede: "Scholarships and financial aid available to students, and how to apply.", sections: NONE },
    PageDef { path: "/student-life/anti-ragging", title: "Anti-ragging cell", lede: "How to report ragging and who will respond.", sections: NONE },
    PageDef { path: "/alumni", title: "Alumni", lede: "Stay connected with the college and with fellow graduates.", sections: NONE },
    PageDef { path: "/iqac", title: "IQAC", lede: "The Internal Quality Assurance Cell: reports, minutes and accreditation documents.", sections: NONE },
    PageDef { path: "/placement", title: "Placement", lede: "Campus recruitment, training and the placement cell.", sections: NONE },
    PageDef { path: "/gallery", title: "Gallery", lede: "Photographs from campus, events and celebrations.", sections: NONE },
    PageDef { path: "/research", title: "Research", lede: "Research programmes in electronics and computer applications.", sections: NONE },
    PageDef { path: "/rti", title: "Right to Information", lede: "How to file a request under the Right to Information Act.", sections: NONE },
    PageDef { path: "/fees", title: "Fees", lede: "Fee structure for each programme.", sections: NONE },
    PageDef { path: "/hub/login", title: "Student Hub", lede: "Your timetable, attendance, marks and notices in one place. Sign-in opens soon.", sections: NONE },
];
