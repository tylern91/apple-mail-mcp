function run(argv) {
  const request = JSON.parse(argv[0]);
  try {
    const mailbox = findMailbox(request.mailbox);
    const message = findMessage(mailbox, request.message_id);
    if (message === null) {
      throw new Error("message " + request.message_id + " not found in mailbox");
    }
    message.flaggedStatus = request.flagged;
    return JSON.stringify({ ok: true });
  } catch (e) {
    return JSON.stringify({ ok: false, error: String(e) });
  }
}

function findMailbox(address) {
  const mail = Application("Mail");
  // "On My Mac" local mailboxes are not under any account object in Mail's JXA model — they
  // live directly on the top-level Mail.mailboxes collection.
  let container;
  if (address.account_name === "On My Mac") {
    container = mail;
  } else {
    const accounts = mail.accounts.whose({ name: address.account_name })();
    if (accounts.length === 0) {
      throw new Error("no account named " + address.account_name);
    }
    container = accounts[0];
  }
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
