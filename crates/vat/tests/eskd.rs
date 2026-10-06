use doris_company::domain::OrgNr;
use doris_ledger::vat_box::VatBox;
use doris_vat::domain::Boxes;
use doris_vat::eskd::{eskd_xml, file_name, vat_number};
use doris_vat::period::VatPeriod;

fn q3() -> VatPeriod {
    VatPeriod { start: "2026-07-01".parse().unwrap(), end: "2026-09-30".parse().unwrap() }
}

fn org() -> OrgNr {
    OrgNr::parse("556016-0680").unwrap()
}

#[test]
fn the_file_lists_boxes_that_are_not_zero_in_skatteverkets_order_and_always_49() {
    let b = |n: u32| VatBox::parse(n).unwrap();
    let boxes = Boxes {
        amounts: vec![(b(5), 412_300), (b(10), 95_000), (b(21), 4_800), (b(30), 1_200), (b(48), 25_810), (b(42), -1_500)],
        vat_due: 70_390,
    };
    assert_eq!(
        eskd_xml(&org(), q3(), &boxes),
        "<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?>\n\
         <eSKDUpload Version=\"6.0\">\n\
         <OrgNr>556016-0680</OrgNr>\n\
         <Moms>\n\
         <Period>202609</Period>\n\
         <ForsMomsEjAnnan>412300</ForsMomsEjAnnan>\n\
         <InkopTjanstAnnatEg>4800</InkopTjanstAnnatEg>\n\
         <ForsOvrigt>-1500</ForsOvrigt>\n\
         <MomsUtgHog>95000</MomsUtgHog>\n\
         <MomsInkopUtgHog>1200</MomsInkopUtgHog>\n\
         <MomsIngAvdr>25810</MomsIngAvdr>\n\
         <MomsBetala>70390</MomsBetala>\n\
         </Moms>\n\
         </eSKDUpload>\n"
    );
    let empty = eskd_xml(&org(), q3(), &Boxes::default());
    assert!(empty.contains("<MomsBetala>0</MomsBetala>"));
    assert!(empty.is_ascii(), "ASCII is valid ISO-8859-1");
}

#[test]
fn the_vat_number_and_file_name_come_from_the_org_nr() {
    assert_eq!(vat_number(&org()), "SE556016068001");
    assert_eq!(file_name(&org(), q3()), "moms_5560160680_202609.xml");
}
