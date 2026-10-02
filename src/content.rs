//! Placeholder content taken from the current nsscrky.ac.in homepage.
//! Layer 3 replaces every function here with SQLx-backed services
//! (and `&'static str` fields become `String` via `sqlx::FromRow`).

pub struct Notice {
    pub title: &'static str,
    pub date: &'static str,
    pub href: &'static str,
    pub is_new: bool,
}

pub struct NewsItem {
    pub id: u32,
    pub body: &'static str,
    pub title: &'static str,
    pub date: &'static str,
    pub href: &'static str,
    pub image: Option<&'static str>,
}

pub struct Programme {
    pub slug: &'static str,
    pub name: &'static str,
    pub department: &'static str,
    pub level: &'static str,
    pub summary: &'static str,
    pub href: &'static str,
}

pub struct RankHolder {
    pub name: &'static str,
    pub rank: &'static str,
    pub department: &'static str,
    pub year: &'static str,
    pub initials: &'static str,
    pub photo: Option<&'static str>,
}

pub struct Unit {
    pub name: &'static str,
    pub full_name: &'static str,
    pub text: &'static str,
    pub values: &'static str,
    pub href: &'static str,
    pub image: &'static str,
}

pub struct Facility {
    pub name: &'static str,
    pub text: &'static str,
}

pub struct Milestone {
    pub when: &'static str,
    pub text: &'static str,
}

pub fn notices() -> Vec<Notice> {
    vec![
        Notice {
            title: "Admissions open for 2026–30: apply for undergraduate programmes",
            date: "Admissions",
            href: "/admissions",
            is_new: true,
        },
        Notice {
            title: "Notice board update",
            date: "Notice",
            href: "/notices",
            is_new: false,
        },
    ]
}

pub fn news() -> Vec<NewsItem> {
    vec![
        NewsItem {
            title: "International Yoga Day",
            date: "29 Jun 2026",
            href: "/news/10",
            id: 10,
            body: "",
            image: Some("https://nsscrky.ac.in/uploads/news/1782723188_57b7230e7c1704b211c8.jpeg"),
        },
        NewsItem {
            title: "Rajakumari N.S.S. College declared a Green Campus",
            date: "17 May 2026",
            href: "/news/7",
            id: 7,
            body: "",
            image: Some("https://nsscrky.ac.in/uploads/news/1778998882_afbc4a587fef3a07fc5b.jpg"),
        },
        NewsItem {
            title: "Chief Minister's mega quiz",
            date: "17 May 2026",
            href: "/news/6",
            id: 6,
            body: "",
            image: Some("https://nsscrky.ac.in/uploads/news/1778998807_6f56aa0d407f53479565.jpeg"),
        },
    ]
}

pub fn programmes() -> Vec<Programme> {
    vec![
        Programme {
            name: "Bachelor of Computer Applications",
            department: "Computer Applications",
            level: "4-year honours",
            summary: "Software and computing foundations, with room for independent thinking on real technical problems.",
            href: "/academics/bca",
            slug: "bca",
        },
        Programme {
            name: "B.Sc. Electronics",
            department: "Electronics",
            level: "4-year honours",
            summary: "Digital and analog electronics, programming, network analysis and electromagnetics for the communication industry.",
            href: "/academics/bsc-electronics",
            slug: "bsc-electronics",
        },
        Programme {
            name: "Bachelor of Business Administration",
            department: "Business Administration",
            level: "4-year honours",
            summary: "Finance, economics, operations, marketing and accounting, with practical training.",
            href: "/academics/bba",
            slug: "bba",
        },
        Programme {
            name: "B.Com (Co-operation), Model I",
            department: "Commerce",
            level: "4-year honours",
            summary: "Commerce degree built on book-keeping and accountancy at Plus Two level.",
            href: "/academics/bcom-cooperation",
            slug: "bcom-cooperation",
        },
        Programme {
            name: "B.Com Computer Applications, Model II",
            department: "Commerce",
            level: "4-year honours",
            summary: "Accounting, finance and business management combined with programming and database skills.",
            href: "/academics/bcom-ca",
            slug: "bcom-ca",
        },
        Programme {
            name: "M.Sc. Electronics",
            department: "Electronics",
            level: "2-year postgraduate",
            summary: "Design and operation of electronic systems, with specialisation in computing and telecommunications.",
            href: "/academics/msc-electronics",
            slug: "msc-electronics",
        },
    ]
}

pub fn rank_holders() -> Vec<RankHolder> {
    vec![
        RankHolder {
            name: "Aparna Prasad",
            rank: "3",
            department: "Computer Applications",
            year: "2025",
            initials: "AP",
            photo: Some("https://nsscrky.ac.in/uploads/toppers/1779001373_b57d21d0fa258fab1122.png"),
        },
        RankHolder {
            name: "Athulya Mohan",
            rank: "6",
            department: "Commerce",
            year: "2025",
            initials: "AM",
            photo: Some("https://nsscrky.ac.in/uploads/toppers/1779000941_616a233022109612abc4.png"),
        },
        RankHolder {
            name: "Aparna Shaji",
            rank: "7",
            department: "Commerce",
            year: "2025",
            initials: "AS",
            photo: Some("https://nsscrky.ac.in/uploads/toppers/1779001178_948e2dd34bff4e7a428f.png"),
        },
    ]
}

pub fn units() -> Vec<Unit> {
    vec![
        Unit {
            name: "NCC",
            full_name: "National Cadet Corps",
            text: "A youth development movement that builds discipline, leadership and a sense of social responsibility through classroom learning and practical training.",
            values: "Leadership, discipline and character, community service",
            href: "/student-life/ncc",
            image: "https://nsscrky.ac.in/uploads/carousel/1779448436_41898e75d1e7aafa5607.png",
        },
        Unit {
            name: "NSS",
            full_name: "National Service Scheme",
            text: "A voluntary programme under the Ministry of Youth Affairs and Sports that gives students a platform for community service and personal growth.",
            values: "Personality development, unity, service to the community",
            href: "/student-life/nss",
            image: "https://nsscrky.ac.in/uploads/carousel/1779450929_d2690612d1be7d04deba.png",
        },
    ]
}

pub fn facilities() -> Vec<Facility> {
    vec![
        Facility { name: "Central library", text: "Textbooks, journals and digital resources, with a reading hall." },
        Facility { name: "Computer laboratories", text: "High-speed internet and current software environments." },
        Facility { name: "Electronics lab", text: "Modern instruments for practical experiments." },
        Facility { name: "Sports facilities", text: "Multi-sport grounds, a gymnasium and athletics training areas." },
        Facility { name: "Seminar hall", text: "Air-conditioned, with AV facilities for seminars and workshops." },
        Facility { name: "Student support cells", text: "Anti-ragging, women's cell, career guidance, counselling and grievance redressal." },
        Facility { name: "Canteen and hostel", text: "A hygienic canteen and hostel rooms for outstation students." },
    ]
}

pub fn milestones() -> Vec<Milestone> {
    vec![
        Milestone { when: "16 January 1995", text: "Foundation stone of the permanent building laid by Sri. P.K. Narayana Panicker." },
        Milestone { when: "June 1995", text: "The college opens in rented premises in Rajakumari town under the Nair Service Society." },
        Milestone { when: "7 March 2000", text: "The hilltop campus near Kulapparachal is inaugurated." },
        Milestone { when: "2002 onwards", text: "B.Com with Computer Applications is introduced and programmes continue to expand." },
    ]
}

pub struct Department {
    pub slug: &'static str,
    pub name: &'static str,
    pub summary: &'static str,
}

pub fn departments() -> Vec<Department> {
    vec![
        Department {
            slug: "computer-applications",
            name: "Computer Applications",
            summary: "Offers the Bachelor of Computer Applications and hosts research in computer applications.",
        },
        Department {
            slug: "electronics",
            name: "Electronics",
            summary: "Offers B.Sc. and M.Sc. Electronics, with research in electronics.",
        },
        Department {
            slug: "business-administration",
            name: "Business Administration",
            summary: "Offers the Bachelor of Business Administration programme.",
        },
        Department {
            slug: "commerce",
            name: "Commerce",
            summary: "Offers B.Com (Co-operation) and B.Com with Computer Applications.",
        },
    ]
}

pub fn programme_by_slug(slug: &str) -> Option<Programme> {
    programmes().into_iter().find(|p| p.slug == slug)
}

pub fn department_by_slug(slug: &str) -> Option<Department> {
    departments().into_iter().find(|d| d.slug == slug)
}

pub fn news_by_id(id: u32) -> Option<NewsItem> {
    news().into_iter().find(|n| n.id == id)
}
