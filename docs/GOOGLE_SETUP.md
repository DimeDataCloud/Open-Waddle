# Connecting Waddle to Google

Once connected, Waddle can:
- read and tidy your Gmail
- check and book your Google Calendar
- look up your Google Contacts

| What Waddle does | What you see |
|---|---|
| Reading mail, your calendar or contacts | Nothing to approve |
| Drafts, archiving, labels, new events, invitations, RSVPs | A 2-second notice you can cancel |
| Sending, binning mail, deleting events | A card that waits for your click |

When Waddle sends an email:
- The send card shows the whole message, and **Edit** lets you change it.
- After **Send**, you have 10 seconds to **Undo**.

Waddle talks to Google directly from your computer, using an OAuth client you create in your own Google Cloud project. Nothing goes through a Waddle server, because there isn't one. Setup takes about 5 minutes.

## 1. Create the Google Cloud project

1. Open [console.cloud.google.com](https://console.cloud.google.com/) signed in with the account you'll connect.
2. Create a project and call it **Waddle**.
3. Open **APIs & Services → Library** and enable these three:
   - **Gmail API**
   - **Google Calendar API**
   - **People API**

## 2. Set up the consent screen

Open **APIs & Services → OAuth consent screen** (sometimes called **Google Auth Platform → Branding / Audience**).

**If you use Google Workspace** (a company or school account), choose **Internal**. This is the recommended setup:
- Google doesn't need to review the app.
- Your sign-in doesn't expire after 7 days.
- Only accounts in your organisation can use it.

**If you use a personal @gmail.com account**, choose **External** instead. Leave it in **Testing** and add yourself under **Test users**. While it's in Testing:
- Google ends the sign-in after **7 days**, so you'll need to press **Connect** again once a week.
- At sign-in, Google warns that it "hasn't verified this app". That's expected for your own project: choose **Continue**.

For either type:
- App name: **Waddle**.
- Support and developer email: your own address.
- You don't need to add scopes here, because Waddle asks for them when you connect.

## 3. Create the OAuth client

1. Open **APIs & Services → Credentials → Create credentials → OAuth client ID**.
2. For **Application type**, choose **Desktop app**. Other types won't work, because Waddle uses a local sign-in redirect.
3. Name it "Waddle desktop" and create it.
4. Copy the **Client ID** and **Client secret**.

Google calls it a secret, but an installed app can't really keep one. What protects your sign-in is PKCE (a one-time code that only this app knows) and the redirect to your own computer.

## 4. Connect in Waddle

1. Right-click the duck, open **Settings**, and find **Google account**.
2. Paste the client ID and the client secret.
3. Press **Connect**. Your browser opens Google's sign-in page.
4. Pick your account and allow access to:
   - Gmail (read, send, organise)
   - Calendar events
   - Contacts (read-only)
5. The browser tab says "Waddle is connected", and Settings shows **Connected as you@…**.

Try it: "what's my next meeting?", "any important unread email?", "reply to Sam that Thursday works", "find 45 minutes with Ana next week".

To teach Waddle your email style, say "learn how I write emails". It reads about 20 of your sent emails once and writes a short note (greeting, sign-off, tone). You can edit the note in Settings.

## Privacy

- **Where the sign-in is kept:** the refresh token (what keeps Waddle signed in) and the client secret are stored in Windows Credential Manager, not in files.
- **Where your data goes:** email, calendar and contact details go from Google to your computer. From there, only what a task needs goes to the AI model you picked. On OpenRouter, Waddle asks for providers that don't keep or train on it.
- **Untrusted content:** mail and event text is marked as untrusted. Waddle is told never to follow instructions found in an email.
- **Disconnecting:** **Disconnect** tells Google to revoke Waddle's access and deletes the token. You can also remove access at [myaccount.google.com/permissions](https://myaccount.google.com/permissions).

## Without Google

If you don't connect, Waddle still helps with email and calendar. It works in Gmail and Google Calendar in Chrome on screen, the same way you would. That's slower, and every click gets its usual notice.

## Troubleshooting

| What you see | What to do |
|---|---|
| `Error 400: redirect_uri_mismatch` | The client isn't a **Desktop app**. Create a new OAuth client with that type. |
| `Error 403: org_internal` | The app is Internal and you signed in with an account outside your organisation. Use your work account, or switch the app to External. |
| `access_denied` / "app is being tested" | Add your address under **Test users** (External apps). |
| "sign-in has expired or was revoked" | Press **Connect** again. External apps in Testing expire every 7 days. |
| "insufficient authentication scopes" | Press **Disconnect**, then **Connect**, and tick every box Google offers. |
| A Gmail, Calendar or People API "has not been used in project" error | Enable that API (step 1) and wait a minute. |
