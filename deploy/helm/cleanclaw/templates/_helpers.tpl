{{/* Common labels */}}
{{- define "cleanclaw.labels" -}}
app.kubernetes.io/name: {{ include "cleanclaw.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
helm.sh/chart: {{ .Chart.Name }}-{{ .Chart.Version }}
{{- end }}

{{/* Selector labels (stable across upgrades) */}}
{{- define "cleanclaw.selectorLabels" -}}
app.kubernetes.io/name: {{ include "cleanclaw.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{/* Chart name */}}
{{- define "cleanclaw.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" -}}
{{- end }}

{{/* Fully qualified app name */}}
{{- define "cleanclaw.fullname" -}}
{{- if .Values.fullnameOverride -}}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- $name := default .Chart.Name .Values.nameOverride -}}
{{- if contains $name .Release.Name -}}
{{- .Release.Name | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" -}}
{{- end -}}
{{- end -}}
{{- end }}
