//! Make a large PST to try the importer on: `make_big <path> <batches>
//! <messages per batch> <attachment KB>`. Not part of the client.

use pst_writer::{UnicodePstAttachment, UnicodePstBatchMessage, UnicodePstFile, UnicodePstMessage};

fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let number = |at: usize, otherwise: usize| {
        args.get(at)
            .and_then(|n| n.parse().ok())
            .unwrap_or(otherwise)
    };
    let path = args.get(1).cloned().unwrap_or_else(|| "big.pst".to_owned());
    let (batches, each, kilobytes) = (number(2, 4), number(3, 500), number(4, 512));

    // Bytes that do not repeat, so nothing stores them smaller than they are.
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    let data: Vec<u8> = (0..kilobytes * 1024)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state as u8
        })
        .collect();
    let first = UnicodePstMessage {
        subject: "The first of many",
        sender_name: "Sam Taylor",
        sender_email: "sam@example.test",
        recipients: &[],
        body: "The first message in a large file.",
        html_body: None,
        message_id: "<first@example.test>",
        delivery_time: 133_750_656_000_000_000,
    };
    drop(UnicodePstFile::create(&path, &first)?);

    for batch in 0..batches {
        let subjects: Vec<String> = (0..each)
            .map(|n| format!("Message {n} of batch {batch} about the harbour"))
            .collect();
        let ids: Vec<String> = (0..each)
            .map(|n| format!("<{batch}-{n}@example.test>"))
            .collect();
        let attachments = [UnicodePstAttachment {
            filename: "survey.bin",
            mime_type: "application/octet-stream",
            content_id: None,
            data: &data,
        }];
        let messages: Vec<UnicodePstBatchMessage<'_>> = subjects
            .iter()
            .zip(&ids)
            .map(|(subject, id)| UnicodePstBatchMessage {
                message: UnicodePstMessage {
                    subject,
                    message_id: id,
                    body: "Piling survey results are attached, as discussed on site.",
                    ..first
                },
                attachments: &attachments,
            })
            .collect();
        drop(UnicodePstFile::append_many_with_attachments(
            &path, &messages,
        )?);
        eprintln!("batch {} of {batches} written", batch + 1);
    }

    Ok(())
}
