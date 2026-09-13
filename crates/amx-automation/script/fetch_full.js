function run(argv) {
  const request = JSON.parse(argv[0]);
  try {
    const mailbox = findMailbox(request.mailbox);
    const message = findMessage(mailbox, request.message_id);
    if (message === null) {
      throw new Error("message " + request.message_id + " not found in mailbox");
    }
    // Reading `content` forces Mail to fetch the full body/attachments for a partial download —
    // there is no separate "download full message" verb in Mail's scripting dictionary.
    void message.content();
    return JSON.stringify({ ok: true });
  } catch (e) {
    return JSON.stringify({ ok: false, error: String(e) });
  }
}

function findMailbox(address) {
  const mail = Application("Mail");
  const accounts = mail.accounts.whose({ name: address.account_name })();
  if (accounts.length === 0) {
    throw new Error("no account named " + address.account_name);
  }
  let container = accounts[0];
  for (const segment of address.mailbox_segments) {
    const matches = container.mailboxes.whose({ name: segment })();
    if (matches.length === 0) {
      throw new Error(
        "no mailbox named " + segment + " under " + address.account_name
      );
    }
    container = matches[0];
  }
  return container;
}

function findMessage(mailbox, messageId) {
  const matches = mailbox.messages.whose({ messageId: messageId })();
  return matches.length > 0 ? matches[0] : null;
}
