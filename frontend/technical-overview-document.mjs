// Pre-render this public reading page with the same components as the browser.
// Its title, description and content are available on the first HTTP response.
export async function technicalOverviewDocument(template) {
  const { TechnicalOverviewPage, OVERVIEW_PATH, OVERVIEW_TITLE, OVERVIEW_DESCRIPTION } = await import('./src/technical-overview.js');
  return template
    .replace(/<title>[^<]*<\/title>/, `<title>${OVERVIEW_TITLE}</title>`)
    .replace(/<meta name="description" content="[^"]*"\s*\/>/, `<meta name="description" content="${OVERVIEW_DESCRIPTION}" />`)
    .replace('</head>', `    <link rel="canonical" href="${OVERVIEW_PATH}" />\n  </head>`)
    .replace('<div id="app"></div>', `<div id="app">${TechnicalOverviewPage()}</div>`);
}
