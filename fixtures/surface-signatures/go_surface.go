package surface

type Incident struct {
	ID string `json:"id"`
}

type Reporter struct {
	Base string
}

type Notifier interface {
	Notify(message string) error
}

func (r *Reporter) Summarize(
	incidents []*Incident,
	limit int,
) ([]string, error) {
	return nil, nil
}

func (r *Reporter) Merge(values ...string) string {
	return ""
}
