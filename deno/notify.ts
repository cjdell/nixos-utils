// PORT=8888 NOTIFY_URL="http://192.168.49.1:8123/api/services/notify/mobile_app_hd1913" PAYLOAD_FORMAT="home_assistant" HEADER_FILE="/home/cjdell/Projects/nixos-utils/deno/header.txt" deno task notify
// curl -X POST http://localhost:8888 -H 'Content-Type: application/json' -d '{"message":"Hello World!","title":"Notification Test"}'

const options = readEnvironment();

Deno.serve({ port: options.port }, async (req, { remoteAddr }) => {
  try {
    if (req.method !== "POST") {
      throw new Error("Validation: Must be a POST request!");
    }

    console.log("Incoming message from:", remoteAddr.hostname);

    if (req.headers.get("content-type") !== "application/json") {
      throw new Error("Validation: Invalid request body!");
    }

    const body = await getBodyAsString(await req.blob());

    const contents: unknown = JSON.parse(body);

    await sendNotification(contents);

    return new Response("Done", { status: 200 });
  } catch (err: unknown) {
    console.log("Error:", err);

    if (err instanceof Error) {
      if (err.message.startsWith("Validation:")) {
        return new Response(err.message, { status: 400 });
      } else {
        return new Response(err.message, { status: 500 });
      }
    } else {
      return new Response("Unknown error", { status: 500 });
    }
  }
});

function readEnvironment() {
  const PORT = Deno.env.get("PORT");
  if (!PORT) throw new Error("No PORT specified!");

  // http://127.0.0.1:8123/api/services/notify/mobile_app_hd1913
  const NOTIFY_URL = Deno.env.get("NOTIFY_URL");
  if (!NOTIFY_URL) throw new Error("No NOTIFY_URL specified!");

  const PAYLOAD_FORMAT = Deno.env.get("PAYLOAD_FORMAT");
  if (!PAYLOAD_FORMAT) throw new Error("No PAYLOAD_FORMAT specified!");

  const HEADER_FILE = Deno.env.get("HEADER_FILE");
  if (!HEADER_FILE) throw new Error("No HEADER_FILE specified!");

  const port = parseInt(PORT, 10);

  const notifyUrl = new URL(NOTIFY_URL);

  if (PAYLOAD_FORMAT !== "home_assistant" && PAYLOAD_FORMAT !== "slack") {
    throw new Error("Invalid PAYLOAD_FORMAT: " + PAYLOAD_FORMAT);
  }

  const payloadFormat = PAYLOAD_FORMAT;

  const headerContents = Deno.readTextFileSync(HEADER_FILE);

  const headerArray = headerContents.split(":").map((str) => str.trim());
  if (headerArray.length !== 2) {
    throw new Error("Invalid header: " + headerContents);
  }

  const header = [headerArray[0], headerArray[1]] as const;

  return { port, notifyUrl, payloadFormat, header } as const;
}

function getBodyAsString(blob: Blob) {
  return new Promise<string>((resolve) => {
    const fr = new FileReader();
    fr.readAsText(blob);
    fr.onload = () => resolve(fr.result as string);
  });
}

async function sendNotification(contents: unknown) {
  if (
    !(typeof contents === "object") || !contents ||
    !("title" in contents) || typeof contents.title !== "string" ||
    !("message" in contents) || typeof contents.message !== "string"
  ) {
    throw new Error("Validation: Invalid request body!");
  }

  const headers: string[][] = [options.header.slice()];

  let body: string;

  if (options.payloadFormat === "home_assistant") {
    body = JSON.stringify({
      title: contents.title,
      message: contents.message,
    });
  } else if (options.payloadFormat === "slack") {
    body = JSON.stringify({
      data: `**${contents.title}**\n${contents.message}`,
    });
  } else {
    throw new Error("Unreachable:" + options.payloadFormat);
  }

  const response = await fetch(options.notifyUrl, {
    method: "POST",
    headers,
    body,
  });

  if (response.status < 200 || response.status >= 300) {
    throw new Error(
      `Error sending notification: [${response.status}] ${response.statusText}`,
    );
  }
}
