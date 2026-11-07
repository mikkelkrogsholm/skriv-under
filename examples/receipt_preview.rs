//! Generate a fictional agreement and its signing appendix for visual review.
use document_signing::{db, pdf};
use lopdf::{
    Document, Stream,
    content::{Content, Operation},
    dictionary,
};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args()
        .nth(1)
        .unwrap_or("var/receipt-preview".into());
    std::fs::create_dir_all(&output)?;
    let mut doc = Document::with_version("1.7");
    let pages = doc.new_object_id();
    let font =
        doc.add_object(dictionary! {"Type"=>"Font","Subtype"=>"Type1","BaseFont"=>"Helvetica"});
    let content = Content {
        operations: vec![
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec!["F1".into(), 20.into()]),
            Operation::new("Td", vec![50.into(), 760.into()]),
            Operation::new(
                "Tj",
                vec![lopdf::Object::string_literal(
                    "FIKTIVT EKSEMPEL - ingen reel aftale",
                )],
            ),
            Operation::new("ET", vec![]),
        ],
    }
    .encode()?;
    let stream = doc.add_object(Stream::new(dictionary! {}, content));
    let page=doc.add_object(dictionary!{"Type"=>"Page","Parent"=>pages,"MediaBox"=>vec![0.into(),0.into(),595.into(),842.into()],"Resources"=>dictionary!{"Font"=>dictionary!{"F1"=>font}},"Contents"=>stream});
    doc.objects.insert(
        pages,
        lopdf::Object::Dictionary(
            dictionary! {"Type"=>"Pages","Kids"=>vec![page.into()],"Count"=>1},
        ),
    );
    let root = doc.add_object(dictionary! {"Type"=>"Catalog","Pages"=>pages});
    doc.trailer.set("Root", root);
    let mut original = Vec::new();
    doc.save_to(&mut original)?;
    let people = ["Anna Eksempel", "Bo Eksempel"]
        .iter()
        .enumerate()
        .map(|(i, name)| db::Recipient {
            id: i as i64 + 1,
            document_id: 42,
            name: name.to_string(),
            email: None,
            status: "signed".into(),
            signed_name: Some(name.to_string()),
            signed_at: Some(1_794_048_000 + i as i64 * 75),
            reason: None,
            consent: Some(db::CONSENT.into()),
        })
        .collect::<Vec<_>>();
    let (signed, receipt) = pdf::complete(
        &original,
        42,
        "Fiktiv prøveaftale",
        &db::hash(&original),
        &people,
    )
    .map_err(std::io::Error::other)?;
    std::fs::write(format!("{output}/signeret-eksempel.pdf"), signed)?;
    std::fs::write(format!("{output}/signeringsbilag.pdf"), receipt)?;
    println!("Fiktive eksempel-PDF'er gemt i {output}");
    Ok(())
}
