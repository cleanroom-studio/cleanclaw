{{- /* DSN: prefer externalDSN, else fall back to bundled postgres. */ -}}
{{- define "cleanclaw.dsn" -}}
{{- if .Values.externalDSN -}}
{{ .Values.externalDSN }}
{{- else if .Values.postgres.enabled -}}
postgres://cleanclaw:{{ required "postgres.password is required when postgres.enabled=true" .Values.postgres.password }}@{{ include "cleanclaw.fullname" . }}-db:5432/cleanclaw?sslmode=disable
{{- else -}}
{{- fail "Either externalDSN or postgres.enabled must be set" -}}
{{- end -}}
{{- end -}}
