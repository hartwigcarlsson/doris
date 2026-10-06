//! Skatteverket's momsdeklaration (SKV 4700) as the page draws it: its
//! sections, headings and row texts word for word, in its two columns.

pub struct Row {
    pub vat_box: u32,
    pub label: &'static str,
}

pub struct Section {
    pub letter: char,
    pub title: &'static str,
    pub rows: &'static [Row],
    /// In the form's right-hand column.
    pub right: bool,
}

const fn row(vat_box: u32, label: &'static str) -> Row {
    Row { vat_box, label }
}

pub const SECTIONS: [Section; 9] = [
    Section {
        letter: 'A',
        title: "Momspliktig försäljning eller uttag exklusive moms",
        right: false,
        rows: &[
            row(
                5,
                "Momspliktig försäljning som inte ingår i ruta 06, 07 eller 08",
            ),
            row(6, "Momspliktiga uttag"),
            row(7, "Beskattningsunderlag vid vinstmarginalbeskattning"),
            row(8, "Hyresinkomster vid frivillig skattskyldighet"),
        ],
    },
    Section {
        letter: 'B',
        title: "Utgående moms på försäljning eller uttag i ruta 05–08",
        right: true,
        rows: &[
            row(10, "Utgående moms 25 %"),
            row(11, "Utgående moms 12 %"),
            row(12, "Utgående moms 6 %"),
        ],
    },
    Section {
        letter: 'C',
        title: "Momspliktiga inköp vid omvänd skattskyldighet",
        right: false,
        rows: &[
            row(20, "Inköp av varor från ett annat EU-land"),
            row(
                21,
                "Inköp av tjänster från ett annat EU-land enligt huvudregeln",
            ),
            row(22, "Inköp av tjänster från ett land utanför EU"),
            row(
                23,
                "Inköp av varor i Sverige som köparen är skattskyldig för",
            ),
            row(
                24,
                "Övriga inköp av tjänster i Sverige som köparen är skattskyldig för",
            ),
        ],
    },
    Section {
        letter: 'D',
        title: "Utgående moms på inköp i ruta 20–24",
        right: true,
        rows: &[
            row(30, "Utgående moms 25 %"),
            row(31, "Utgående moms 12 %"),
            row(32, "Utgående moms 6 %"),
        ],
    },
    Section {
        letter: 'H',
        title: "Import",
        right: false,
        rows: &[row(50, "Beskattningsunderlag vid import")],
    },
    Section {
        letter: 'I',
        title: "Utgående moms på import i ruta 50",
        right: true,
        rows: &[
            row(60, "Utgående moms 25 %"),
            row(61, "Utgående moms 12 %"),
            row(62, "Utgående moms 6 %"),
        ],
    },
    Section {
        letter: 'E',
        title: "Försäljning m.m. som är undantagen från moms",
        right: false,
        rows: &[
            row(35, "Försäljning av varor till ett annat EU-land"),
            row(36, "Försäljning av varor utanför EU"),
            row(37, "Mellanmans inköp av varor vid trepartshandel"),
            row(38, "Mellanmans försäljning av varor vid trepartshandel"),
            row(
                39,
                "Försäljning av tjänster till näringsidkare i annat EU-land enligt huvudregeln",
            ),
            row(40, "Övrig försäljning av tjänster omsatta utanför Sverige"),
            row(41, "Försäljning när köparen är skattskyldig i Sverige"),
            row(42, "Övrig försäljning m.m."),
        ],
    },
    Section {
        letter: 'F',
        title: "Ingående moms",
        right: true,
        rows: &[row(48, "Ingående moms att dra av")],
    },
    Section {
        letter: 'G',
        title: "Moms att betala eller få tillbaka",
        right: true,
        rows: &[row(49, "Moms att betala eller få tillbaka")],
    },
];

pub fn box_label(n: u32) -> Option<&'static str> {
    SECTIONS
        .iter()
        .flat_map(|s| s.rows)
        .find(|r| r.vat_box == n)
        .map(|r| r.label)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_form_has_every_box_once_left_and_right_as_skv_4700() {
        let mut boxes: Vec<u32> = SECTIONS
            .iter()
            .flat_map(|s| s.rows.iter().map(|r| r.vat_box))
            .collect();
        boxes.sort_unstable();
        let mut expected = vec![
            5, 6, 7, 8, 10, 11, 12, 20, 21, 22, 23, 24, 30, 31, 32, 35, 36, 37, 38, 39, 40, 41, 42,
            48, 49, 50, 60, 61, 62,
        ];
        expected.sort_unstable();
        assert_eq!(boxes, expected);
        let left: String = SECTIONS
            .iter()
            .filter(|s| !s.right)
            .map(|s| s.letter)
            .collect();
        let right: String = SECTIONS
            .iter()
            .filter(|s| s.right)
            .map(|s| s.letter)
            .collect();
        assert_eq!((left.as_str(), right.as_str()), ("ACHE", "BDIFG"));
        assert_eq!(box_label(48), Some("Ingående moms att dra av"));
        assert_eq!(box_label(49), Some("Moms att betala eller få tillbaka"));
        assert_eq!(box_label(13), None);
    }
}
