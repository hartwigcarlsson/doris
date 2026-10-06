//! The momsdeklaration as Skatteverket's eSKD file (eSKDUpload 6.0), which
//! the user uploads in Skatteverket's e-tjänst. Written as text: only tags
//! and digits, so ASCII and thus valid ISO-8859-1.

use crate::domain::Boxes;
use crate::period::VatPeriod;
use doris_company::domain::OrgNr;

/// Each box's element, in the file's order. 49 (`MomsBetala`) comes last.
const ELEMENTS: [(u8, &str); 28] = [
    (5, "ForsMomsEjAnnan"),
    (6, "UttagMoms"),
    (7, "UlagMargbesk"),
    (8, "HyrinkomstFriv"),
    (20, "InkopVaruAnnatEg"),
    (21, "InkopTjanstAnnatEg"),
    (22, "InkopTjanstUtomEg"),
    (23, "InkopVaruSverige"),
    (24, "InkopTjanstSverige"),
    (50, "MomsUlagImport"),
    (35, "ForsVaruAnnatEg"),
    (36, "ForsVaruUtomEg"),
    (37, "InkopVaruMellan3p"),
    (38, "ForsVaruMellan3p"),
    (39, "ForsTjSkskAnnatEg"),
    (40, "ForsTjOvrUtomEg"),
    (41, "ForsKopareSkskSverige"),
    (42, "ForsOvrigt"),
    (10, "MomsUtgHog"),
    (11, "MomsUtgMedel"),
    (12, "MomsUtgLag"),
    (30, "MomsInkopUtgHog"),
    (31, "MomsInkopUtgMedel"),
    (32, "MomsInkopUtgLag"),
    (60, "MomsImportUtgHog"),
    (61, "MomsImportUtgMedel"),
    (62, "MomsImportUtgLag"),
    (48, "MomsIngAvdr"),
];

pub fn eskd_xml(org_nr: &OrgNr, period: VatPeriod, boxes: &Boxes) -> String {
    let mut xml = format!(
        "<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?>\n<eSKDUpload Version=\"6.0\">\n\
         <OrgNr>{}</OrgNr>\n<Moms>\n<Period>{}</Period>\n",
        org_nr.formatted(),
        period.code()
    );
    for (n, tag) in ELEMENTS {
        let kr = boxes.get(n);
        if kr != 0 {
            xml.push_str(&format!("<{tag}>{kr}</{tag}>\n"));
        }
    }
    xml.push_str(&format!(
        "<MomsBetala>{}</MomsBetala>\n</Moms>\n</eSKDUpload>\n",
        boxes.vat_due
    ));
    xml
}

/// SE, the ten digits, 01. For an enskild firma that is the owner's
/// personnummer, as Skatteverket assigns it.
pub fn vat_number(org_nr: &OrgNr) -> String {
    format!("SE{}01", org_nr.as_str())
}

pub fn file_name(org_nr: &OrgNr, period: VatPeriod) -> String {
    format!("moms_{}_{}.xml", org_nr.as_str(), period.code())
}
