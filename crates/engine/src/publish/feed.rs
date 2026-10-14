//! The podcast feed: RSS 2.0 with the Apple Podcasts (itunes:) tags Apple and Spotify read, and the
//! Podcasting 2.0 tags for chapters and transcripts. Each publish adds an item to a kept list and
//! writes the whole feed again.

use serde::{Deserialize, Serialize};

/// The show.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Channel {
    pub title: String,
    pub description: String,
    pub author: String,
    /// Contact address Apple asks for (itunes:owner), may be empty.
    pub email: String,
    /// Website of the show (the feed's <link>).
    pub link: String,
    /// Public URL of the cover (square, 1400–3000 px).
    pub image: String,
    /// ISO 639-1, e.g. "de".
    pub language: String,
    /// Apple category, e.g. "Society & Culture".
    pub category: String,
    pub explicit: bool,
    /// Public URL of the feed itself (atom:link rel=self).
    pub feed_url: String,
}

/// One episode in the feed.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Item {
    pub guid: String,
    pub title: String,
    pub description: String,
    /// Public URL, size in bytes and MIME type of the audio.
    pub audio_url: String,
    pub audio_bytes: u64,
    pub audio_type: String,
    pub duration_s: u64,
    /// RFC 2822 date.
    pub pub_date: String,
    pub episode: u32,
    pub season: u32,
    pub image: String,
    pub chapters_url: String,
    pub transcript_url: String,
}

/// What a feed keeps between publishes.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Feed {
    pub channel: Channel,
    /// Newest first.
    pub items: Vec<Item>,
}

fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&apos;"),
            // Control characters are not allowed in XML 1.0.
            c if (c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r') => {}
            c => o.push(c),
        }
    }
    o
}

fn hms(s: u64) -> String {
    format!("{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}

impl Feed {
    /// Adds `item` (replacing one with the same guid) newest first.
    pub fn add(&mut self, item: Item) {
        self.items.retain(|i| i.guid != item.guid);
        self.items.insert(0, item);
    }

    /// The feed as XML.
    pub fn to_xml(&self) -> String {
        let c = &self.channel;
        let mut x = String::new();
        x.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
        x.push_str("<rss version=\"2.0\" xmlns:itunes=\"http://www.itunes.com/dtds/podcast-1.0.dtd\" xmlns:podcast=\"https://podcastindex.org/namespace/1.0\" xmlns:atom=\"http://www.w3.org/2005/Atom\" xmlns:content=\"http://purl.org/rss/1.0/modules/content/\">\n<channel>\n");
        x.push_str(&format!("  <title>{}</title>\n", esc(&c.title)));
        x.push_str(&format!("  <description>{}</description>\n", esc(&c.description)));
        x.push_str(&format!("  <itunes:summary>{}</itunes:summary>\n", esc(&c.description)));
        if !c.link.is_empty() {
            x.push_str(&format!("  <link>{}</link>\n", esc(&c.link)));
        }
        if !c.feed_url.is_empty() {
            x.push_str(&format!("  <atom:link href=\"{}\" rel=\"self\" type=\"application/rss+xml\"/>\n", esc(&c.feed_url)));
        }
        x.push_str(&format!("  <language>{}</language>\n", esc(if c.language.is_empty() { "de" } else { &c.language })));
        x.push_str(&format!("  <itunes:author>{}</itunes:author>\n", esc(&c.author)));
        if !c.email.is_empty() {
            x.push_str(&format!(
                "  <itunes:owner><itunes:name>{}</itunes:name><itunes:email>{}</itunes:email></itunes:owner>\n",
                esc(&c.author),
                esc(&c.email)
            ));
        }
        if !c.image.is_empty() {
            x.push_str(&format!("  <itunes:image href=\"{}\"/>\n", esc(&c.image)));
            x.push_str(&format!("  <image><url>{}</url><title>{}</title><link>{}</link></image>\n", esc(&c.image), esc(&c.title), esc(&c.link)));
        }
        if !c.category.is_empty() {
            x.push_str(&format!("  <itunes:category text=\"{}\"/>\n", esc(&c.category)));
        }
        x.push_str(&format!("  <itunes:explicit>{}</itunes:explicit>\n", c.explicit));
        x.push_str("  <itunes:type>episodic</itunes:type>\n");
        x.push_str("  <generator>TalkCraft</generator>\n");
        for i in &self.items {
            x.push_str("  <item>\n");
            x.push_str(&format!("    <title>{}</title>\n", esc(&i.title)));
            x.push_str(&format!("    <description>{}</description>\n", esc(&i.description)));
            x.push_str(&format!("    <content:encoded>{}</content:encoded>\n", esc(&i.description)));
            x.push_str(&format!("    <guid isPermaLink=\"false\">{}</guid>\n", esc(&i.guid)));
            x.push_str(&format!("    <pubDate>{}</pubDate>\n", esc(&i.pub_date)));
            x.push_str(&format!("    <enclosure url=\"{}\" length=\"{}\" type=\"{}\"/>\n", esc(&i.audio_url), i.audio_bytes, esc(&i.audio_type)));
            x.push_str(&format!("    <itunes:duration>{}</itunes:duration>\n", hms(i.duration_s)));
            if i.episode > 0 {
                x.push_str(&format!("    <itunes:episode>{}</itunes:episode>\n", i.episode));
            }
            if i.season > 0 {
                x.push_str(&format!("    <itunes:season>{}</itunes:season>\n", i.season));
            }
            x.push_str("    <itunes:episodeType>full</itunes:episodeType>\n");
            if !i.image.is_empty() {
                x.push_str(&format!("    <itunes:image href=\"{}\"/>\n", esc(&i.image)));
            }
            if !i.chapters_url.is_empty() {
                x.push_str(&format!("    <podcast:chapters url=\"{}\" type=\"application/json+chapters\"/>\n", esc(&i.chapters_url)));
            }
            if !i.transcript_url.is_empty() {
                x.push_str(&format!("    <podcast:transcript url=\"{}\" type=\"text/vtt\"/>\n", esc(&i.transcript_url)));
            }
            x.push_str("  </item>\n");
        }
        x.push_str("</channel>\n</rss>\n");
        x
    }
}

/// RFC 2822 date of a Unix time (UTC), e.g. "Tue, 14 Oct 2026 12:00:00 +0000".
pub fn rfc2822(unix: i64) -> String {
    let days = unix.div_euclid(86_400);
    let secs = unix.rem_euclid(86_400);
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    let wd = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"][(days.rem_euclid(7)) as usize];
    let mon = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"][(m - 1).clamp(0, 11) as usize];
    format!("{wd}, {d:02} {mon} {y} {:02}:{:02}:{:02} +0000", secs / 3600, secs / 60 % 60, secs % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_are_rfc_2822() {
        assert_eq!(rfc2822(0), "Thu, 01 Jan 1970 00:00:00 +0000");
        assert_eq!(rfc2822(1_791_979_200), "Wed, 14 Oct 2026 12:00:00 +0000");
    }

    #[test]
    fn the_feed_has_the_tags_apple_spotify_and_podcasting_2_read_and_escapes_text() {
        let mut f = Feed {
            channel: Channel {
                title: "Europa 2.0 & mehr".into(),
                description: "Gespräche <über> Europa".into(),
                author: "Dirk Gerber".into(),
                image: "https://x.example/cover.jpg".into(),
                language: "de".into(),
                category: "Society & Culture".into(),
                feed_url: "https://x.example/feed.xml".into(),
                ..Default::default()
            },
            items: Vec::new(),
        };
        let item = |n: u32| Item {
            guid: format!("g{n}"),
            title: format!("Folge {n}"),
            audio_url: format!("https://x.example/{n}.m4a"),
            audio_bytes: 1000,
            audio_type: "audio/mp4".into(),
            duration_s: 3725,
            pub_date: rfc2822(0),
            episode: n,
            chapters_url: format!("https://x.example/{n}.chapters.json"),
            ..Default::default()
        };
        f.add(item(1));
        f.add(item(2));
        f.add(item(1));
        assert_eq!(f.items.iter().map(|i| i.episode).collect::<Vec<_>>(), [1, 2], "a re-publish replaces, newest first");
        let x = f.to_xml();
        for tag in [
            "<title>Europa 2.0 &amp; mehr</title>",
            "Gespräche &lt;über&gt; Europa",
            "<itunes:image href=\"https://x.example/cover.jpg\"/>",
            "<itunes:category text=\"Society &amp; Culture\"/>",
            "<enclosure url=\"https://x.example/1.m4a\" length=\"1000\" type=\"audio/mp4\"/>",
            "<itunes:duration>01:02:05</itunes:duration>",
            "<podcast:chapters url=\"https://x.example/1.chapters.json\" type=\"application/json+chapters\"/>",
            "<atom:link href=\"https://x.example/feed.xml\" rel=\"self\"",
        ] {
            assert!(x.contains(tag), "missing {tag}\n{x}");
        }
    }
}
