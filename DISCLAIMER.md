# Disclaimer

## No Warranty

This software is provided "AS IS", without warranty of any kind, express or implied, including but not limited to the warranties of merchantability, fitness for a particular purpose, and non-infringement. The entire risk as to the quality and performance of the software is with you.

## Limitation of Liability

In no event shall the authors or copyright holders be liable for any claim, damages, or other liability, whether in an action of contract, tort, or otherwise, arising from, out of, or in connection with the software or the use or other dealings in the software. This includes, without limitation, any direct, indirect, incidental, special, exemplary, or consequential damages (including but not limited to loss of data, loss of profits, or business interruption).

## Precompiled Binaries

Precompiled binaries published on the [releases page](https://github.com/tylern91/apple-mail-mcp/releases) are provided solely for convenience and are covered by the same license as the source code (MIT). They are provided without warranties or conditions of any kind. You are responsible for verifying the integrity and suitability of any binary before use. Release tags are GPG-signed — verify the tag signature, or the checksum published alongside each binary, before running it.

## Third-Party Dependencies

This software incorporates third-party open-source components, each governed by their respective licenses. The authors make no representations or warranties regarding these dependencies and accept no liability for any issues arising from their use.

## Use at Your Own Risk

This software reads Apple Mail's on-disk store (the Envelope Index SQLite database and `.emlx` message files under `~/Library/Mail`) directly, drives Mail.app via Apple Events/Automation to move, flag, delete, and send mail on your behalf, and — if you enable the MCP HTTP server — can expose that access over the network (see [SECURITY.md](SECURITY.md) for the listener's authentication requirements). It is your responsibility to ensure that its use, and how you configure it, is appropriate for your environment and complies with any applicable policies, regulations, or agreements. The authors are not responsible for any unintended side effects resulting from its use, including mail sent, moved, or deleted as a result of granting an LLM client access to these tools.

## Data and Network Access — no telemetry

apple-mail-mcp collects and transmits **no usage metrics, telemetry, or analytics of any kind**. There is no opt-out setting because there is nothing to opt out of.

The software's only network activity is functional, not data collection:

- **SMTP delivery.** Sending a message connects to the mail server configured in your existing Mail.app account settings, exactly as Mail.app itself would.
- **Local MCP listener.** The MCP server binds to `127.0.0.1` by default (see [SECURITY.md](SECURITY.md) for the requirements to bind beyond loopback).

No mail content, metadata, file paths, or command arguments are sent anywhere except to your own configured mail server, as a direct result of the actions you request.

---

See [LICENSE](LICENSE) for the full terms of the MIT License under which this software is distributed.
