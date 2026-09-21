// Package skills holds the skills the Harness ships: start-work and one Stage
// skill per Stage. 'harness init' copies them into a Target repo.
package skills

import "embed"

//go:embed */SKILL.md
var FS embed.FS
