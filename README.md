# Diana's Amazing Invoice Bot (WIP)

Using this requires my specific invoice template, so, lol.

It currently expect exactly one spreadsheet named "Invoice Template"
and folder named "Test" to exist.

This application will copy the template to Test,
rename it to `Invoice-(iso date)`, put todays date in cells `D9:E9`,
and then export the sheet as `./scratch/test.pdf`

It expects the information from a google provided `client_secret.json`
in environment variables
