{{- define "murmur-server.name" -}}
{{- .Chart.Name | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "murmur-server.fullname" -}}
{{- if contains .Chart.Name .Release.Name -}}
{{- .Release.Name | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- printf "%s-%s" .Release.Name .Chart.Name | trunc 63 | trimSuffix "-" -}}
{{- end -}}
{{- end -}}

{{- define "murmur-server.labels" -}}
helm.sh/chart: {{ printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" }}
{{ include "murmur-server.selectorLabels" . }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end -}}

{{- define "murmur-server.selectorLabels" -}}
app.kubernetes.io/name: {{ include "murmur-server.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end -}}

{{/* config.toml: values.config, plus [tls] when serverTls is on. */}}
{{- define "murmur-server.config" -}}
{{ .Values.config }}
{{- if .Values.serverTls.enabled }}

[tls]
cert = "/etc/murmur-server/tls/tls.crt"
key = "/etc/murmur-server/tls/tls.key"
{{- end }}
{{- end -}}

{{- define "murmur-server.image" -}}
{{- $img := .Values.image -}}
{{- if .Values.gpu.enabled -}}{{- $img = merge (dict) .Values.gpu.image .Values.image -}}{{- end -}}
{{- printf "%s:%s" $img.repository $img.tag -}}
{{- end -}}
