// Auto-tagger plugin for CXMail
// Tags emails based on sender patterns

var rules = [
  { pattern: "github.com", tag: "dev" },
  { pattern: "newsletter", tag: "newsletter" },
  { pattern: "noreply", tag: "automated" },
  { pattern: "receipt", tag: "finance" },
  { pattern: "invoice", tag: "finance" },
  { pattern: "shipping", tag: "shopping" },
  { pattern: "tracking", tag: "shopping" },
];

if (__trigger === "on_new_email") {
  var email = __triggerData;

  if (typeof email === "string") {
    try { email = JSON.parse(email); } catch (e) { email = {}; }
  }

  var from = (email.from_email || "").toLowerCase();
  var subject = (email.subject || "").toLowerCase();

  for (var i = 0; i < rules.length; i++) {
    var rule = rules[i];
    if (from.indexOf(rule.pattern) !== -1 || subject.indexOf(rule.pattern) !== -1) {
      cxmail.log("Auto-tagged as: " + rule.tag);
      if (email.uid) {
        cxmail.flagEmail(email.uid, "starred");
      }
      break;
    }
  }
}
