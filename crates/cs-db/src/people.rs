//! People the user has named, with face fingerprints to recognise them again.
//!
//! Privacy: a sample is a 128-number fingerprint, an optional year and an
//! optional small face crop. Photo paths are never stored, and nothing here
//! ever leaves the PC. `delete_person` removes every sample of that person.

use crate::{Db, Result};
use rusqlite::{OptionalExtension, params};

/// Samples kept per person: enough variety (ages, angles) for recognition,
/// small enough that comparing stays instant.
pub const MAX_SAMPLES: usize = 300;

/// A named person's fingerprints: (person id, name, fingerprints).
pub type KnownPerson = (i64, String, Vec<Vec<f32>>);

#[derive(Debug, Clone, PartialEq)]
pub struct PersonRow {
    pub id: i64,
    pub name: String,
    pub samples: usize,
    /// Distinct years the person was seen in, ascending.
    pub years: Vec<i16>,
    /// Optional, set by the user.
    pub birth_year: Option<i16>,
}

/// One face to remember.
#[derive(Debug, Clone, Default)]
pub struct Sample {
    pub embedding: Vec<f32>,
    pub year: Option<i16>,
    /// Small JPEG of the face (shown in the app).
    pub crop: Option<Vec<u8>>,
}

fn to_blob(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

fn from_blob(b: &[u8]) -> Vec<f32> {
    b.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect()
}

/// Names are shown everywhere, so keep them sane: trimmed, 1..=60 characters,
/// no control characters.
pub fn clean_name(name: &str) -> std::result::Result<String, &'static str> {
    let n: String = name.split_whitespace().collect::<Vec<_>>().join(" ");
    if n.is_empty() {
        return Err("Please type a name.");
    }
    if n.chars().count() > 60 {
        return Err("Please use a shorter name (60 characters at most).");
    }
    if n.chars().any(char::is_control) {
        return Err("The name contains characters that cannot be used.");
    }
    Ok(n)
}

impl Db {
    pub fn add_person(&self, name: &str) -> Result<i64> {
        self.conn.execute("INSERT INTO people (name) VALUES (?1)", [name])?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn rename_person(&self, id: i64, name: &str) -> Result<bool> {
        Ok(self.conn.execute("UPDATE people SET name = ?2 WHERE id = ?1", params![id, name])? == 1)
    }

    /// Forget a person and all their samples.
    pub fn delete_person(&self, id: i64) -> Result<bool> {
        self.batch(|db| {
            db.conn.execute("DELETE FROM face_samples WHERE person_id = ?1", [id])?;
            Ok(db.conn.execute("DELETE FROM people WHERE id = ?1", [id])? == 1)
        })
    }

    /// `other` turns out to be the same person as `keep`: move the samples over.
    pub fn merge_people(&self, keep: i64, other: i64) -> Result<bool> {
        if keep == other || self.person_name(keep)?.is_none() || self.person_name(other)?.is_none() {
            return Ok(false);
        }
        self.batch(|db| {
            db.conn.execute("UPDATE face_samples SET person_id = ?1 WHERE person_id = ?2", params![keep, other])?;
            db.conn.execute("DELETE FROM people WHERE id = ?1", [other])?;
            db.trim_samples(keep)?;
            Ok(true)
        })
    }

    /// Set or clear (None) a person's birth year.
    pub fn set_birth_year(&self, id: i64, year: Option<i16>) -> Result<bool> {
        Ok(self.conn.execute("UPDATE people SET birth_year = ?2 WHERE id = ?1", params![id, year])? == 1)
    }

    pub fn person_name(&self, id: i64) -> Result<Option<String>> {
        Ok(self.conn.query_row("SELECT name FROM people WHERE id = ?1", [id], |r| r.get(0)).optional()?)
    }

    /// Person with this name (case-insensitive), if any.
    pub fn person_by_name(&self, name: &str) -> Result<Option<i64>> {
        Ok(self
            .conn
            .query_row("SELECT id FROM people WHERE lower(name) = lower(?1) ORDER BY id LIMIT 1", [name], |r| r.get(0))
            .optional()?)
    }

    /// Remember faces of a person. Returns how many were added (at most
    /// [`MAX_SAMPLES`] are kept per person; the oldest go first).
    pub fn add_samples(&self, person_id: i64, samples: &[Sample]) -> Result<usize> {
        self.batch(|db| {
            let mut st = db
                .conn
                .prepare("INSERT INTO face_samples (person_id, embedding, year, crop) VALUES (?1, ?2, ?3, ?4)")?;
            for s in samples {
                st.execute(params![person_id, to_blob(&s.embedding), s.year, s.crop])?;
            }
            db.trim_samples(person_id)?;
            Ok(samples.len().min(MAX_SAMPLES))
        })
    }

    fn trim_samples(&self, person_id: i64) -> Result<()> {
        self.conn.execute(
            "DELETE FROM face_samples WHERE person_id = ?1 AND id NOT IN
               (SELECT id FROM face_samples WHERE person_id = ?1 ORDER BY id DESC LIMIT ?2)",
            params![person_id, MAX_SAMPLES as i64],
        )?;
        Ok(())
    }

    /// Everyone, by name.
    pub fn people(&self) -> Result<Vec<PersonRow>> {
        let mut st = self.conn.prepare(
            "SELECT p.id, p.name, COUNT(s.id), p.birth_year,
                    (SELECT group_concat(y, ',') FROM
                        (SELECT DISTINCT year AS y FROM face_samples WHERE person_id = p.id AND year IS NOT NULL ORDER BY y))
             FROM people p LEFT JOIN face_samples s ON s.person_id = p.id
             GROUP BY p.id ORDER BY lower(p.name), p.id",
        )?;
        let rows = st.query_map([], |r| {
            let years: Option<String> = r.get(4)?;
            Ok(PersonRow {
                id: r.get(0)?,
                name: r.get(1)?,
                samples: r.get::<_, i64>(2)? as usize,
                years: years.unwrap_or_default().split(',').filter_map(|y| y.parse().ok()).collect(),
                birth_year: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// All fingerprints, grouped per person: (person id, name, fingerprints).
    pub fn all_samples(&self) -> Result<Vec<KnownPerson>> {
        let mut out: Vec<KnownPerson> = Vec::new();
        let mut st = self.conn.prepare(
            "SELECT p.id, p.name, s.embedding FROM face_samples s JOIN people p ON p.id = s.person_id ORDER BY p.id, s.id",
        )?;
        let mut rows = st.query([])?;
        while let Some(r) = rows.next()? {
            let (id, name, blob): (i64, String, Vec<u8>) = (r.get(0)?, r.get(1)?, r.get(2)?);
            match out.last_mut() {
                Some(last) if last.0 == id => last.2.push(from_blob(&blob)),
                _ => out.push((id, name, vec![from_blob(&blob)])),
            }
        }
        Ok(out)
    }

    /// The newest face crop of a person (for the picture next to the name).
    pub fn person_cover(&self, id: i64) -> Result<Option<Vec<u8>>> {
        Ok(self
            .conn
            .query_row(
                "SELECT crop FROM face_samples WHERE person_id = ?1 AND crop IS NOT NULL ORDER BY id DESC LIMIT 1",
                [id],
                |r| r.get(0),
            )
            .optional()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: f32, year: Option<i16>) -> Sample {
        Sample { embedding: vec![v, 1.0 - v], year, crop: Some(vec![v as u8]) }
    }

    #[test]
    fn people_round_trip() {
        let db = Db::open_in_memory().unwrap();
        let a = db.add_person("Anna").unwrap();
        let b = db.add_person("Ben").unwrap();
        db.add_samples(a, &[s(0.25, Some(1999)), s(0.5, Some(1995)), s(0.75, Some(1999))]).unwrap();
        db.add_samples(b, &[s(0.0, None)]).unwrap();
        let p = db.people().unwrap();
        assert_eq!(
            p[0],
            PersonRow { id: a, name: "Anna".into(), samples: 3, years: vec![1995, 1999], birth_year: None }
        );
        db.set_birth_year(a, Some(1990)).unwrap();
        assert_eq!(db.people().unwrap()[0].birth_year, Some(1990));
        assert_eq!(p[1].years, Vec::<i16>::new());
        let all = db.all_samples().unwrap();
        assert_eq!(all[0].2[1], vec![0.5, 0.5], "fingerprints survive the blob round trip");
        assert_eq!(db.person_by_name("anna").unwrap(), Some(a));

        assert!(db.merge_people(a, b).unwrap());
        assert_eq!(db.people().unwrap().len(), 1);
        assert_eq!(db.people().unwrap()[0].samples, 4);
        assert!(!db.merge_people(a, a).unwrap(), "cannot merge someone with themselves");

        assert!(db.rename_person(a, "Anna K").unwrap());
        assert!(db.delete_person(a).unwrap());
        let left: i64 = db.conn().query_row("SELECT COUNT(*) FROM face_samples", [], |r| r.get(0)).unwrap();
        assert_eq!(left, 0, "forgetting a person removes every sample");
    }

    #[test]
    fn samples_are_capped_oldest_first() {
        let db = Db::open_in_memory().unwrap();
        let a = db.add_person("A").unwrap();
        let many: Vec<Sample> = (0..MAX_SAMPLES + 5).map(|i| s(i as f32, None)).collect();
        db.add_samples(a, &many).unwrap();
        let all = db.all_samples().unwrap();
        assert_eq!(all[0].2.len(), MAX_SAMPLES);
        assert_eq!(all[0].2[0][0], 5.0, "the oldest samples were dropped");
    }

    #[test]
    fn names_are_cleaned() {
        assert_eq!(clean_name("  Anna   Maria ").unwrap(), "Anna Maria");
        assert!(clean_name("   ").is_err());
        assert!(clean_name(&"x".repeat(61)).is_err());
    }
}
