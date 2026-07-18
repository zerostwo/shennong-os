type RoutePolicy = { pattern: RegExp; methods: ReadonlySet<string> };

const methods = (...values: string[]): ReadonlySet<string> => new Set(values);
const READ = methods("GET", "HEAD");
const READ_CREATE = methods("GET", "HEAD", "POST");

// This is intentionally narrower than the server router. In particular, the
// service-authenticated /agent/runs/** callback surface is never reachable
// through the browser BFF. Additions to the OS API must be reviewed here.
const ROUTES: readonly RoutePolicy[] = [
  { pattern: /^(?:setup\/status|public-config|capabilities)$/, methods: READ },
  { pattern: /^setup\/admin$/, methods: methods("POST") },
  { pattern: /^auth\/registration-policy$/, methods: READ },
  { pattern: /^auth\/(?:register|sign-in|sign-out)$/, methods: methods("POST") },
  { pattern: /^auth\/session$/, methods: READ },
  { pattern: /^auth\/profile$/, methods: methods("PATCH") },
  { pattern: /^auth\/sessions$/, methods: READ },
  { pattern: /^auth\/sessions\/[^/]+$/, methods: methods("DELETE") },

  { pattern: /^admin\/invites$/, methods: READ_CREATE },
  { pattern: /^admin\/invites\/[^/]+$/, methods: methods("DELETE") },
  { pattern: /^admin\/registration-policy$/, methods: methods("PATCH") },
  { pattern: /^admin\/(?:overview|model-providers)$/, methods: READ },
  { pattern: /^users$/, methods: READ },
  { pattern: /^users\/[^/]+$/, methods: methods("GET", "HEAD", "PUT") },

  { pattern: /^projects$/, methods: READ_CREATE },
  { pattern: /^projects\/[^/]+$/, methods: methods("GET", "HEAD", "PATCH") },
  { pattern: /^projects\/[^/]+\/members$/, methods: READ },
  { pattern: /^projects\/[^/]+\/members\/[^/]+$/, methods: methods("PUT", "DELETE") },
  { pattern: /^projects\/[^/]+\/jobs$/, methods: READ_CREATE },
  { pattern: /^projects\/[^/]+\/sessions$/, methods: READ_CREATE },
  { pattern: /^projects\/[^/]+\/artifacts$/, methods: READ_CREATE },
  { pattern: /^projects\/[^/]+\/uploads$/, methods: READ_CREATE },
  { pattern: /^projects\/[^/]+\/uploads\/register$/, methods: methods("POST") },
  { pattern: /^projects\/[^/]+\/context-pack$/, methods: READ },
  { pattern: /^projects\/[^/]+\/graph\/subgraph$/, methods: READ },
  {
    pattern: /^projects\/[^/]+\/(?:entities|activities|studies|associations|evidence)(?:\/[^/]+){0,3}$/,
    methods: methods("GET", "HEAD", "POST", "PUT", "DELETE"),
  },
  {
    pattern: /^projects\/[^/]+\/resources(?:\/[^/]+){0,3}$/,
    methods: methods("GET", "HEAD", "PUT", "DELETE"),
  },

  { pattern: /^threads$/, methods: READ_CREATE },
  { pattern: /^threads\/[^/]+$/, methods: methods("GET", "HEAD", "PATCH", "DELETE") },
  { pattern: /^threads\/[^/]+\/messages$/, methods: READ_CREATE },
  { pattern: /^threads\/[^/]+\/runs$/, methods: methods("POST") },
  { pattern: /^threads\/[^/]+\/runs\/active$/, methods: READ },
  { pattern: /^threads\/[^/]+\/skills$/, methods: READ },
  { pattern: /^threads\/[^/]+\/skills\/[^/]+$/, methods: methods("PUT", "DELETE") },
  { pattern: /^runs$/, methods: READ },
  { pattern: /^runs\/[^/]+$/, methods: methods("GET", "HEAD", "PATCH") },
  { pattern: /^runs\/[^/]+\/events$/, methods: READ_CREATE },
  { pattern: /^runs\/[^/]+\/events\/stream$/, methods: READ },
  { pattern: /^runs\/[^/]+\/plan$/, methods: methods("GET", "HEAD", "PUT") },

  { pattern: /^jobs$/, methods: READ },
  { pattern: /^jobs\/[^/]+$/, methods: methods("GET", "HEAD", "PATCH") },
  { pattern: /^jobs\/[^/]+\/cancel$/, methods: methods("POST") },
  { pattern: /^sessions\/[^/]+$/, methods: READ },
  { pattern: /^sessions\/[^/]+\/(?:stop|launch)$/, methods: methods("POST") },

  { pattern: /^memories$/, methods: READ_CREATE },
  { pattern: /^memories\/[^/]+$/, methods: methods("GET", "HEAD", "PATCH", "DELETE") },
  { pattern: /^skills$/, methods: READ_CREATE },
  { pattern: /^skills\/[^/]+$/, methods: methods("GET", "HEAD", "PATCH") },
  { pattern: /^skills\/[^/]+\/versions$/, methods: READ_CREATE },
  { pattern: /^providers$/, methods: READ_CREATE },
  { pattern: /^providers\/[^/]+$/, methods: methods("PATCH", "DELETE") },

  { pattern: /^system\/dependencies$/, methods: READ },
  { pattern: /^audit-events$/, methods: READ },
  { pattern: /^resources$/, methods: READ },
  { pattern: /^resource-providers$/, methods: READ },
  { pattern: /^resources\/install$/, methods: methods("POST") },
  { pattern: /^resources\/[^/]+$/, methods: READ },
  { pattern: /^resources\/[^/]+\/(?:artifacts|relations|graph-context)$/, methods: READ },
  { pattern: /^query$/, methods: methods("POST") },
];

export function isBrowserRouteAllowed(path: string, method: string): boolean {
  const normalizedMethod = method.toUpperCase();
  return ROUTES.some((route) => route.pattern.test(path) && route.methods.has(normalizedMethod));
}
