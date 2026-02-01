{{/*
Expand the name of the chart.
*/}}
{{- define "cleo.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Create a default fully qualified app name.
*/}}
{{- define "cleo.fullname" -}}
{{- if .Values.fullnameOverride }}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- $name := default .Chart.Name .Values.nameOverride }}
{{- if contains $name .Release.Name }}
{{- .Release.Name | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" }}
{{- end }}
{{- end }}
{{- end }}

{{/*
Create chart name and version as used by the chart label.
*/}}
{{- define "cleo.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Common labels
*/}}
{{- define "cleo.labels" -}}
helm.sh/chart: {{ include "cleo.chart" . }}
{{ include "cleo.selectorLabels" . }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end }}

{{/*
Selector labels
*/}}
{{- define "cleo.selectorLabels" -}}
app.kubernetes.io/name: {{ include "cleo.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{/*
API image
*/}}
{{- define "cleo.apiImage" -}}
{{- if .Values.image.registry }}
{{- printf "%s/%s:%s" .Values.image.registry .Values.api.image.repository .Values.api.image.tag }}
{{- else }}
{{- printf "%s:%s" .Values.api.image.repository .Values.api.image.tag }}
{{- end }}
{{- end }}

{{/*
Web image
*/}}
{{- define "cleo.webImage" -}}
{{- if .Values.image.registry }}
{{- printf "%s/%s:%s" .Values.image.registry .Values.web.image.repository .Values.web.image.tag }}
{{- else }}
{{- printf "%s:%s" .Values.web.image.repository .Values.web.image.tag }}
{{- end }}
{{- end }}

{{/*
Service account name
*/}}
{{- define "cleo.serviceAccountName" -}}
{{- if .Values.serviceAccount.create }}
{{- default (include "cleo.fullname" .) .Values.serviceAccount.name }}
{{- else }}
{{- default "default" .Values.serviceAccount.name }}
{{- end }}
{{- end }}
