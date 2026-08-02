use anyhow::{Result, bail};

// Common, visually distinct three-letter English words. Keeping this list
// embedded makes word labels deterministic and portable.
const THREE_LETTER_WORDS: &str = "
ace act add age ago aid aim air ale all and ant any ape apt arc are arm art ash ask ate awe axe
bad bag ban bar bat bay bed bee beg bet bid big bin bit bob bog boo bow box boy bud bug bun bus but buy
cab can cap car cat cod cog cop cot cow cry cub cup cut
dad dam day den dew did die dig dim din dip dog dot dry due dug dye
ear eat eel egg ego elf elk elm end era eve eye
fan far fat fax fed fee few fig fin fir fit fix flu fly fog for fox fry fun fur
gab gag gap gas gel gem get gig gin god got gum gun gut guy gym
had ham has hat hay hen her hey hid him hip his hit hog hop hot how hub hug hum hut
ice icy ill ink inn ion its
jam jar jaw jay jet job jog joy jug
key kid kin kit
lab lad lag lap law lay led leg let lid lie lip lit log lot low
mad man map mat may men met mid mix mob mom mop mud mug
nap net new nib nod nor not now nun nut
oak oar odd off oil old one orb ore our out owl own
pad pal pan par pat paw pay pea peg pen pet pie pig pin pit pod pop pot pub pup put
rag ram ran rap rat raw ray red rib rid rig rim rip rob rod rot row rub rug run rye
sad sag sap sat saw say sea see set sew she shy sin sip sir sit six ski sky sly sob son sow spa spy sum sun
tab tag tan tap tar tea ten the tie tin tip toe ton too top toy try tub tug two
urn use
van vat vet
war was wax way web wed wet who why win wit wok won wow
yak yam yes yet you
zap zen zip zoo
";

pub fn generate(alphabet: &str, count: usize) -> Result<Vec<String>> {
    let chars: Vec<char> = alphabet.chars().collect();
    if chars.len() < 2 {
        bail!("alphabet must contain at least two characters");
    }
    if count == 0 {
        return Ok(Vec::new());
    }
    let mut length = 1usize;
    let mut capacity = chars.len();
    while capacity < count {
        length += 1;
        capacity = capacity
            .checked_mul(chars.len())
            .ok_or_else(|| anyhow::anyhow!("grid is too large"))?;
    }

    Ok((0..count)
        .map(|mut n| {
            let mut label = vec![chars[0]; length];
            for slot in label.iter_mut().rev() {
                *slot = chars[n % chars.len()];
                n /= chars.len();
            }
            label.into_iter().collect()
        })
        .collect())
}

pub fn generate_words(count: usize) -> Result<Vec<String>> {
    let words: Vec<&str> = THREE_LETTER_WORDS.split_whitespace().collect();
    if count > words.len() {
        bail!(
            "word-label grid needs {count} labels but only {} are available",
            words.len()
        );
    }
    Ok(words.into_iter().take(count).map(str::to_owned).collect())
}

pub fn word_count() -> usize {
    THREE_LETTER_WORDS.split_whitespace().count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn labels_are_fixed_length_and_unique() {
        let labels = generate("asdf", 16).unwrap();
        assert!(labels.iter().all(|label| label.len() == 2));
        assert_eq!(labels.iter().collect::<HashSet<_>>().len(), 16);
    }

    #[test]
    fn grows_to_three_characters() {
        let labels = generate("ab", 5).unwrap();
        assert!(labels.iter().all(|label| label.len() == 3));
    }

    #[test]
    fn rejects_tiny_alphabet() {
        assert!(generate("a", 5).is_err());
    }

    #[test]
    fn word_labels_are_unique_recognizable_triplets() {
        let labels = generate_words(word_count()).unwrap();
        assert!(labels.len() >= 250);
        assert!(labels.iter().all(|label| label.len() == 3));
        assert_eq!(labels.iter().collect::<HashSet<_>>().len(), labels.len());
        assert!(labels.contains(&"cat".to_owned()));
        assert!(labels.contains(&"you".to_owned()));
    }
}
