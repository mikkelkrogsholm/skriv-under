use crate::db::{self, Recipient};
use lopdf::{
    Document, Object, Stream,
    content::{Content, Operation},
    dictionary,
};

pub fn validate(bytes: &[u8]) -> Result<(), String> {
    if bytes.len() > 20 * 1024 * 1024 || !bytes.starts_with(b"%PDF-") {
        return Err("Vælg en PDF på højst 20 MiB".into());
    }
    let doc = Document::load_mem_with_options(
        bytes,
        lopdf::LoadOptions {
            max_decompressed_size: Some(64 * 1024 * 1024),
            ..Default::default()
        },
    )
    .map_err(|_| "PDF-filen kunne ikke læses")?;
    if doc.is_encrypted() {
        return Err("Adgangskodebeskyttede PDF-filer understøttes ikke".into());
    }
    let pages = doc.get_pages().len();
    if pages == 0 || pages > 200 {
        return Err("PDF-filen skal have mellem 1 og 200 sider".into());
    }
    let tree = doc
        .catalog()
        .and_then(|catalog| catalog.get_deref(b"Pages", &doc))
        .and_then(Object::as_dict)
        .map_err(|_| "PDF-filens sidetræ kunne ikke læses")?;
    tree.get_deref(b"Kids", &doc)
        .and_then(Object::as_array)
        .map_err(|_| "PDF-filens sidetræ kunne ikke læses")?;
    let declared = tree
        .get_deref(b"Count", &doc)
        .and_then(Object::as_i64)
        .map_err(|_| "PDF-filens sideantal kunne ikke læses")?;
    if declared != pages as i64 {
        return Err("PDF-filens sideantal matcher ikke dens sider".into());
    }
    for object in doc.objects.values() {
        let dict = match object {
            Object::Dictionary(d) => Some(d),
            Object::Stream(s) => Some(&s.dict),
            _ => None,
        };
        if let Some(d) = dict {
            for key in [
                b"JS".as_slice(),
                b"JavaScript",
                b"OpenAction",
                b"AA",
                b"EmbeddedFiles",
                b"EF",
                b"XFA",
                b"RichMediaContent",
            ] {
                if d.has(key) {
                    return Err(
                        "PDF-filer med aktivt indhold eller vedhæftede filer understøttes ikke"
                            .into(),
                    );
                }
            }
            if d.get(b"Type")
                .and_then(Object::as_name)
                .is_ok_and(|v| v == b"EmbeddedFile")
                || d.get(b"Subtype")
                    .and_then(Object::as_name)
                    .is_ok_and(|v| v == b"FileAttachment")
            {
                return Err("PDF-filer med vedhæftede filer understøttes ikke".into());
            }
            if d.get(b"S").and_then(Object::as_name).is_ok_and(|v| {
                matches!(v, b"JavaScript" | b"Launch" | b"SubmitForm" | b"ImportData")
            }) || d
                .get(b"FT")
                .and_then(Object::as_name)
                .is_ok_and(|v| v == b"Sig")
            {
                return Err(
                    "PDF-filen indeholder en handling eller en eksisterende certifikatunderskrift"
                        .into(),
                );
            }
        }
    }
    Ok(())
}

// The standard PDF fonts cover Danish and Western European characters. Unsupported
// characters are rendered as numeric references, also preserved literally in audit JSON.
fn text_bytes(s: &str) -> Vec<u8> {
    encoding_rs::WINDOWS_1252.encode(s).0.into_owned()
}
#[derive(Clone)]
struct ReceiptLine {
    text: String,
    signature: bool,
}
impl ReceiptLine {
    fn height(&self) -> i64 {
        if self.signature { 22 } else { 13 }
    }
}
fn wrapped(text: &str, signature: bool) -> Vec<ReceiptLine> {
    // Wrap the actual encoded representation, including expanded numeric references.
    let encoded = text_bytes(text);
    let display = encoding_rs::WINDOWS_1252.decode(&encoded).0;
    let limit = if signature { 35 } else { 90 };
    let mut result = Vec::new();
    let mut line = String::new();
    for word in display.split_whitespace() {
        if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > limit {
            result.push(ReceiptLine {
                text: std::mem::take(&mut line),
                signature,
            });
        }
        if !line.is_empty() {
            line.push(' ');
        }
        for ch in word.chars() {
            if line.chars().count() >= limit {
                result.push(ReceiptLine {
                    text: std::mem::take(&mut line),
                    signature,
                });
            }
            line.push(ch);
        }
    }
    if !line.is_empty() {
        result.push(ReceiptLine {
            text: line,
            signature,
        });
    }
    result
}
fn blank() -> ReceiptLine {
    ReceiptLine {
        text: String::new(),
        signature: false,
    }
}
fn timestamp(at: i64) -> Result<String, String> {
    chrono::DateTime::from_timestamp(at, 0)
        .map(|t| t.format("%Y-%m-%d %H:%M:%S UTC").to_string())
        .ok_or("Ugyldigt signeringstidspunkt".into())
}
fn receipt_pages(
    id: i64,
    title: &str,
    hash: &str,
    original_pages: usize,
    recipients: &[Recipient],
) -> Result<Vec<Vec<ReceiptLine>>, String> {
    let completed = recipients
        .iter()
        .filter_map(|r| r.signed_at)
        .max()
        .ok_or("Signeringstidspunkt mangler")?;
    let mut blocks = Vec::new();
    let mut intro = Vec::new();
    for text in [
        format!("Dokumentreference: SU-{id}"),
        format!("Dokument: {title}"),
        format!(
            "Original PDF: {original_pages} {}",
            if original_pages == 1 { "side" } else { "sider" }
        ),
        format!("Alle underskrifter registreret: {}", timestamp(completed)?),
        "SHA-256 af original PDF (uden signeringsbilag):".into(),
        hash.into(),
        format!(
            "Underskrifter: {} af {}",
            recipients.len(),
            recipients.len()
        ),
    ] {
        intro.extend(wrapped(&text, false));
    }
    intro.push(blank());
    blocks.push(intro);
    for (index, r) in recipients.iter().enumerate() {
        let name = r
            .signed_name
            .as_deref()
            .ok_or("Underskrevet navn mangler")?;
        let consent = r
            .consent
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .ok_or("Det accepterede samtykke mangler")?;
        let mut block = wrapped(
            &format!(
                "UNDERSKRIFT {} AF {} · SU-{id}-{}",
                index + 1,
                recipients.len(),
                r.id
            ),
            false,
        );
        block.extend(wrapped("Elektronisk underskrift (indtastet navn):", false));
        block.extend(wrapped(name, true));
        for text in [
            format!("Tildelt modtager: {}", r.name),
            format!(
                "Bekræftet: {}",
                timestamp(r.signed_at.ok_or("Signeringstidspunkt mangler")?)?
            ),
            "Handling: Underskriv dokumentet + aktiv accept af samtykke.".into(),
            format!("Accepteret samtykke: {consent}"),
        ] {
            block.extend(wrapped(&text, false));
        }
        block.push(blank());
        blocks.push(block);
    }
    let mut method = Vec::new();
    for text in [
        "DOKUMENTATION OG METODE",
        "Personligt signeringslink oprettet af administratoren til manuel deling. Navn, tidspunkt, samtykke og dokumentversion er registreret ved bekræftelsen.",
        "Tidspunkter er registreret af serveren. Linket verificerer ikke personens identitet; der er ingen MitID-kontrol, certifikatbaseret PDF-signatur eller uafhængigt tidsstempel.",
        "Dokumentreference og signerings-ID'er kan sammenholdes med administratorens revisionsspor. Bilaget indeholder ingen hemmelige links eller tokens.",
        "Navne uden for Windows-1252 vises som numeriske referencer; den fulde Unicode-tekst bevares i databasen og revisionssporets JSON-eksport.",
    ] {
        method.extend(wrapped(text, false));
    }
    blocks.push(method);
    const CAPACITY: i64 = 630;
    let mut pages = vec![Vec::new()];
    let mut height = 0;
    for block in blocks {
        let block_height: i64 = block.iter().map(ReceiptLine::height).sum();
        if block_height <= CAPACITY && height > 0 && height + block_height > CAPACITY {
            pages.push(Vec::new());
            height = 0;
        }
        for line in block {
            if height + line.height() > CAPACITY {
                pages.push(Vec::new());
                height = 0;
            }
            height += line.height();
            pages.last_mut().unwrap().push(line);
        }
    }
    Ok(pages)
}
fn draw_text(ops: &mut Vec<Operation>, text: &str, font: &str, size: i64, x: i64, y: i64) {
    ops.extend([
        Operation::new("BT", vec![]),
        Operation::new("Tf", vec![font.into(), size.into()]),
        Operation::new(
            "Tm",
            vec![1.into(), 0.into(), 0.into(), 1.into(), x.into(), y.into()],
        ),
        Operation::new("Tj", vec![Object::string_literal(text_bytes(text))]),
        Operation::new("ET", vec![]),
    ]);
}
fn add_pages(
    doc: &mut Document,
    root: lopdf::ObjectId,
    id: i64,
    pages: &[Vec<ReceiptLine>],
) -> Result<(), String> {
    let regular=doc.add_object(dictionary!{"Type"=>"Font","Subtype"=>"Type1","BaseFont"=>"Courier","Encoding"=>"WinAnsiEncoding"});
    let heading=doc.add_object(dictionary!{"Type"=>"Font","Subtype"=>"Type1","BaseFont"=>"Helvetica-Bold","Encoding"=>"WinAnsiEncoding"});
    let signature=doc.add_object(dictionary!{"Type"=>"Font","Subtype"=>"Type1","BaseFont"=>"Helvetica-Oblique","Encoding"=>"WinAnsiEncoding"});
    let mut kids = Vec::new();
    for (index, lines) in pages.iter().enumerate() {
        let mut ops = vec![Operation::new(
            "rg",
            vec![0.09.into(), 0.36.into(), 0.29.into()],
        )];
        draw_text(&mut ops, "Skriv Under", "F2", 12, 45, 797);
        draw_text(
            &mut ops,
            "Underskrifter og dokumentation",
            "F2",
            20,
            45,
            762,
        );
        ops.push(Operation::new(
            "RG",
            vec![0.75.into(), 0.82.into(), 0.77.into()],
        ));
        ops.push(Operation::new("m", vec![45.into(), 744.into()]));
        ops.push(Operation::new("l", vec![550.into(), 744.into()]));
        ops.push(Operation::new("S", vec![]));
        let mut y = 720;
        for line in lines {
            ops.push(Operation::new(
                "rg",
                if line.signature {
                    vec![0.09.into(), 0.36.into(), 0.29.into()]
                } else {
                    vec![0.12.into(), 0.18.into(), 0.19.into()]
                },
            ));
            draw_text(
                &mut ops,
                &line.text,
                if line.signature { "F3" } else { "F1" },
                if line.signature { 14 } else { 9 },
                45,
                y,
            );
            y -= line.height();
        }
        ops.push(Operation::new(
            "rg",
            vec![0.35.into(), 0.42.into(), 0.39.into()],
        ));
        draw_text(
            &mut ops,
            &format!("SU-{id} | Signeringsbilag {} af {}", index + 1, pages.len()),
            "F1",
            8,
            45,
            40,
        );
        let bytes = Content { operations: ops }
            .encode()
            .map_err(|e| e.to_string())?;
        let content = doc.add_object(Stream::new(dictionary! {}, bytes));
        let bounds = vec![0.into(), 0.into(), 595.into(), 842.into()];
        let page=doc.add_object(dictionary!{"Type"=>"Page","Parent"=>root,"MediaBox"=>bounds.clone(),"CropBox"=>bounds,"Rotate"=>0,"Resources"=>dictionary!{"Font"=>dictionary!{"F1"=>regular,"F2"=>heading,"F3"=>signature}},"Contents"=>content});
        kids.push(Object::Reference(page));
    }
    let tree = doc.get_dictionary(root).map_err(|e| e.to_string())?;
    let current = tree
        .get_deref(b"Count", doc)
        .and_then(Object::as_i64)
        .map_err(|e| e.to_string())?;
    let count = current
        .checked_add(pages.len() as i64)
        .ok_or("PDF-filens sideantal er for stort")?;
    let mut all_kids = tree
        .get_deref(b"Kids", doc)
        .and_then(Object::as_array)
        .map_err(|e| e.to_string())?
        .clone();
    all_kids.extend(kids);
    let tree = doc
        .get_object_mut(root)
        .and_then(Object::as_dict_mut)
        .map_err(|e| e.to_string())?;
    tree.set("Kids", all_kids);
    tree.set("Count", count);
    Ok(())
}
pub fn complete(
    original: &[u8],
    document_id: i64,
    title: &str,
    hash: &str,
    recipients: &[Recipient],
) -> Result<(Vec<u8>, Vec<u8>), String> {
    if recipients.is_empty()
        || recipients.iter().any(|r| {
            r.document_id != document_id
                || r.status != "signed"
                || r.signed_name.as_deref().is_none_or(|n| n.trim().is_empty())
        })
    {
        return Err("Signeringsbilag kræver alle dokumentets registrerede underskrifter".into());
    }
    if db::hash(original) != hash {
        return Err("Original PDF matcher ikke dokumenthashen".into());
    }
    let mut final_doc = Document::load_mem_with_options(
        original,
        lopdf::LoadOptions {
            max_decompressed_size: Some(64 * 1024 * 1024),
            ..Default::default()
        },
    )
    .map_err(|e| e.to_string())?;
    let pages = receipt_pages(
        document_id,
        title,
        hash,
        final_doc.get_pages().len(),
        recipients,
    )?;
    let mut receipt = Document::with_version("1.7");
    let root =
        receipt.add_object(dictionary! {"Type"=>"Pages","Kids"=>Vec::<Object>::new(),"Count"=>0});
    let catalog = receipt.add_object(dictionary! {"Type"=>"Catalog","Pages"=>root});
    receipt.trailer.set("Root", catalog);
    add_pages(&mut receipt, root, document_id, &pages)?;
    let mut receipt_bytes = Vec::new();
    receipt
        .save_to(&mut receipt_bytes)
        .map_err(|e| e.to_string())?;
    let root = final_doc
        .catalog()
        .and_then(|d| d.get(b"Pages"))
        .and_then(Object::as_reference)
        .map_err(|e| e.to_string())?;
    add_pages(&mut final_doc, root, document_id, &pages)?;
    let mut final_bytes = Vec::new();
    final_doc
        .save_to(&mut final_bytes)
        .map_err(|e| e.to_string())?;
    Ok((final_bytes, receipt_bytes))
}
