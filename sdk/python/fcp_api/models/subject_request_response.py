from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast
import datetime

if TYPE_CHECKING:
  from ..models.bundle_response import BundleResponse
  from ..models.erasure_report import ErasureReport
  from ..models.export_report import ExportReport
  from ..models.subject_expansion import SubjectExpansion
  from ..models.subject_identifier_type_0 import SubjectIdentifierType0
  from ..models.subject_identifier_type_1 import SubjectIdentifierType1
  from ..models.subject_identifier_type_2 import SubjectIdentifierType2
  from ..models.subject_identifier_type_3 import SubjectIdentifierType3





T = TypeVar("T", bound="SubjectRequestResponse")



@_attrs_define
class SubjectRequestResponse:
    """ 
        Attributes:
            id (str):
            tenant_id (str):
            kind (str):
            status (str):
            subjects (list[SubjectIdentifierType0 | SubjectIdentifierType1 | SubjectIdentifierType2 |
                SubjectIdentifierType3]):
            requested_by (str):
            received_at (datetime.datetime):
            due_at (datetime.datetime):
            overdue (bool): Open and past `due_at`.
            expansion (None | SubjectExpansion | Unset):
            reference (None | str | Unset):
            started_at (datetime.datetime | None | Unset):
            completed_at (datetime.datetime | None | Unset):
            report (ExportReport | None | Unset):
            bundle (BundleResponse | None | Unset):
            error (None | str | Unset):
            confirmed_at (datetime.datetime | None | Unset):
            confirmed_by (None | str | Unset):
            preview (ErasureReport | None | Unset):
            erasure (ErasureReport | None | Unset):
     """

    id: str
    tenant_id: str
    kind: str
    status: str
    subjects: list[SubjectIdentifierType0 | SubjectIdentifierType1 | SubjectIdentifierType2 | SubjectIdentifierType3]
    requested_by: str
    received_at: datetime.datetime
    due_at: datetime.datetime
    overdue: bool
    expansion: None | SubjectExpansion | Unset = UNSET
    reference: None | str | Unset = UNSET
    started_at: datetime.datetime | None | Unset = UNSET
    completed_at: datetime.datetime | None | Unset = UNSET
    report: ExportReport | None | Unset = UNSET
    bundle: BundleResponse | None | Unset = UNSET
    error: None | str | Unset = UNSET
    confirmed_at: datetime.datetime | None | Unset = UNSET
    confirmed_by: None | str | Unset = UNSET
    preview: ErasureReport | None | Unset = UNSET
    erasure: ErasureReport | None | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.bundle_response import BundleResponse # noqa: PLC0415
        from ..models.erasure_report import ErasureReport # noqa: PLC0415
        from ..models.export_report import ExportReport # noqa: PLC0415
        from ..models.subject_expansion import SubjectExpansion # noqa: PLC0415
        from ..models.subject_identifier_type_0 import SubjectIdentifierType0 # noqa: PLC0415
        from ..models.subject_identifier_type_1 import SubjectIdentifierType1 # noqa: PLC0415
        from ..models.subject_identifier_type_2 import SubjectIdentifierType2 # noqa: PLC0415
        from ..models.subject_identifier_type_3 import SubjectIdentifierType3 # noqa: PLC0415
        id = self.id

        tenant_id = self.tenant_id

        kind = self.kind

        status = self.status

        subjects = []
        for subjects_item_data in self.subjects:
            subjects_item: dict[str, Any]
            if isinstance(subjects_item_data, SubjectIdentifierType0):
                subjects_item = subjects_item_data.to_dict()
            elif isinstance(subjects_item_data, SubjectIdentifierType1):
                subjects_item = subjects_item_data.to_dict()
            elif isinstance(subjects_item_data, SubjectIdentifierType2):
                subjects_item = subjects_item_data.to_dict()
            else:
                subjects_item = subjects_item_data.to_dict()

            subjects.append(subjects_item)



        requested_by = self.requested_by

        received_at = self.received_at.isoformat()

        due_at = self.due_at.isoformat()

        overdue = self.overdue

        expansion: dict[str, Any] | None | Unset
        if isinstance(self.expansion, Unset):
            expansion = UNSET
        elif isinstance(self.expansion, SubjectExpansion):
            expansion = self.expansion.to_dict()
        else:
            expansion = self.expansion

        reference: None | str | Unset
        if isinstance(self.reference, Unset):
            reference = UNSET
        else:
            reference = self.reference

        started_at: None | str | Unset
        if isinstance(self.started_at, Unset):
            started_at = UNSET
        elif isinstance(self.started_at, datetime.datetime):
            started_at = self.started_at.isoformat()
        else:
            started_at = self.started_at

        completed_at: None | str | Unset
        if isinstance(self.completed_at, Unset):
            completed_at = UNSET
        elif isinstance(self.completed_at, datetime.datetime):
            completed_at = self.completed_at.isoformat()
        else:
            completed_at = self.completed_at

        report: dict[str, Any] | None | Unset
        if isinstance(self.report, Unset):
            report = UNSET
        elif isinstance(self.report, ExportReport):
            report = self.report.to_dict()
        else:
            report = self.report

        bundle: dict[str, Any] | None | Unset
        if isinstance(self.bundle, Unset):
            bundle = UNSET
        elif isinstance(self.bundle, BundleResponse):
            bundle = self.bundle.to_dict()
        else:
            bundle = self.bundle

        error: None | str | Unset
        if isinstance(self.error, Unset):
            error = UNSET
        else:
            error = self.error

        confirmed_at: None | str | Unset
        if isinstance(self.confirmed_at, Unset):
            confirmed_at = UNSET
        elif isinstance(self.confirmed_at, datetime.datetime):
            confirmed_at = self.confirmed_at.isoformat()
        else:
            confirmed_at = self.confirmed_at

        confirmed_by: None | str | Unset
        if isinstance(self.confirmed_by, Unset):
            confirmed_by = UNSET
        else:
            confirmed_by = self.confirmed_by

        preview: dict[str, Any] | None | Unset
        if isinstance(self.preview, Unset):
            preview = UNSET
        elif isinstance(self.preview, ErasureReport):
            preview = self.preview.to_dict()
        else:
            preview = self.preview

        erasure: dict[str, Any] | None | Unset
        if isinstance(self.erasure, Unset):
            erasure = UNSET
        elif isinstance(self.erasure, ErasureReport):
            erasure = self.erasure.to_dict()
        else:
            erasure = self.erasure


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "id": id,
            "tenant_id": tenant_id,
            "kind": kind,
            "status": status,
            "subjects": subjects,
            "requested_by": requested_by,
            "received_at": received_at,
            "due_at": due_at,
            "overdue": overdue,
        })
        if expansion is not UNSET:
            field_dict["expansion"] = expansion
        if reference is not UNSET:
            field_dict["reference"] = reference
        if started_at is not UNSET:
            field_dict["started_at"] = started_at
        if completed_at is not UNSET:
            field_dict["completed_at"] = completed_at
        if report is not UNSET:
            field_dict["report"] = report
        if bundle is not UNSET:
            field_dict["bundle"] = bundle
        if error is not UNSET:
            field_dict["error"] = error
        if confirmed_at is not UNSET:
            field_dict["confirmed_at"] = confirmed_at
        if confirmed_by is not UNSET:
            field_dict["confirmed_by"] = confirmed_by
        if preview is not UNSET:
            field_dict["preview"] = preview
        if erasure is not UNSET:
            field_dict["erasure"] = erasure

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.bundle_response import BundleResponse # noqa: PLC0415
        from ..models.erasure_report import ErasureReport # noqa: PLC0415
        from ..models.export_report import ExportReport # noqa: PLC0415
        from ..models.subject_expansion import SubjectExpansion # noqa: PLC0415
        from ..models.subject_identifier_type_0 import SubjectIdentifierType0 # noqa: PLC0415
        from ..models.subject_identifier_type_1 import SubjectIdentifierType1 # noqa: PLC0415
        from ..models.subject_identifier_type_2 import SubjectIdentifierType2 # noqa: PLC0415
        from ..models.subject_identifier_type_3 import SubjectIdentifierType3 # noqa: PLC0415
        d = dict(src_dict)
        id = d.pop("id")

        tenant_id = d.pop("tenant_id")

        kind = d.pop("kind")

        status = d.pop("status")

        subjects = []
        _subjects = d.pop("subjects")
        for subjects_item_data in (_subjects):
            def _parse_subjects_item(data: object) -> SubjectIdentifierType0 | SubjectIdentifierType1 | SubjectIdentifierType2 | SubjectIdentifierType3:
                try:
                    if not isinstance(data, dict):
                        raise TypeError()
                    componentsschemas_subject_identifier_type_0 = SubjectIdentifierType0.from_dict(data)



                    return componentsschemas_subject_identifier_type_0
                except (TypeError, ValueError, AttributeError, KeyError):
                    pass
                try:
                    if not isinstance(data, dict):
                        raise TypeError()
                    componentsschemas_subject_identifier_type_1 = SubjectIdentifierType1.from_dict(data)



                    return componentsschemas_subject_identifier_type_1
                except (TypeError, ValueError, AttributeError, KeyError):
                    pass
                try:
                    if not isinstance(data, dict):
                        raise TypeError()
                    componentsschemas_subject_identifier_type_2 = SubjectIdentifierType2.from_dict(data)



                    return componentsschemas_subject_identifier_type_2
                except (TypeError, ValueError, AttributeError, KeyError):
                    pass
                if not isinstance(data, dict):
                    raise TypeError()
                componentsschemas_subject_identifier_type_3 = SubjectIdentifierType3.from_dict(data)



                return componentsschemas_subject_identifier_type_3

            subjects_item = _parse_subjects_item(subjects_item_data)

            subjects.append(subjects_item)


        requested_by = d.pop("requested_by")

        received_at = datetime.datetime.fromisoformat(d.pop("received_at"))




        due_at = datetime.datetime.fromisoformat(d.pop("due_at"))




        overdue = d.pop("overdue")

        def _parse_expansion(data: object) -> None | SubjectExpansion | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, dict):
                    raise TypeError()
                expansion_type_0 = SubjectExpansion.from_dict(data)



                return expansion_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(None | SubjectExpansion | Unset, data)

        expansion = _parse_expansion(d.pop("expansion", UNSET))


        def _parse_reference(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        reference = _parse_reference(d.pop("reference", UNSET))


        def _parse_started_at(data: object) -> datetime.datetime | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, str):
                    raise TypeError()
                started_at_type_0 = datetime.datetime.fromisoformat(data)



                return started_at_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(datetime.datetime | None | Unset, data)

        started_at = _parse_started_at(d.pop("started_at", UNSET))


        def _parse_completed_at(data: object) -> datetime.datetime | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, str):
                    raise TypeError()
                completed_at_type_0 = datetime.datetime.fromisoformat(data)



                return completed_at_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(datetime.datetime | None | Unset, data)

        completed_at = _parse_completed_at(d.pop("completed_at", UNSET))


        def _parse_report(data: object) -> ExportReport | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, dict):
                    raise TypeError()
                report_type_0 = ExportReport.from_dict(data)



                return report_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(ExportReport | None | Unset, data)

        report = _parse_report(d.pop("report", UNSET))


        def _parse_bundle(data: object) -> BundleResponse | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, dict):
                    raise TypeError()
                bundle_type_0 = BundleResponse.from_dict(data)



                return bundle_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(BundleResponse | None | Unset, data)

        bundle = _parse_bundle(d.pop("bundle", UNSET))


        def _parse_error(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        error = _parse_error(d.pop("error", UNSET))


        def _parse_confirmed_at(data: object) -> datetime.datetime | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, str):
                    raise TypeError()
                confirmed_at_type_0 = datetime.datetime.fromisoformat(data)



                return confirmed_at_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(datetime.datetime | None | Unset, data)

        confirmed_at = _parse_confirmed_at(d.pop("confirmed_at", UNSET))


        def _parse_confirmed_by(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        confirmed_by = _parse_confirmed_by(d.pop("confirmed_by", UNSET))


        def _parse_preview(data: object) -> ErasureReport | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, dict):
                    raise TypeError()
                preview_type_0 = ErasureReport.from_dict(data)



                return preview_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(ErasureReport | None | Unset, data)

        preview = _parse_preview(d.pop("preview", UNSET))


        def _parse_erasure(data: object) -> ErasureReport | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, dict):
                    raise TypeError()
                erasure_type_0 = ErasureReport.from_dict(data)



                return erasure_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(ErasureReport | None | Unset, data)

        erasure = _parse_erasure(d.pop("erasure", UNSET))


        subject_request_response = cls(
            id=id,
            tenant_id=tenant_id,
            kind=kind,
            status=status,
            subjects=subjects,
            requested_by=requested_by,
            received_at=received_at,
            due_at=due_at,
            overdue=overdue,
            expansion=expansion,
            reference=reference,
            started_at=started_at,
            completed_at=completed_at,
            report=report,
            bundle=bundle,
            error=error,
            confirmed_at=confirmed_at,
            confirmed_by=confirmed_by,
            preview=preview,
            erasure=erasure,
        )


        subject_request_response.additional_properties = d
        return subject_request_response

    @property
    def additional_keys(self) -> list[str]:
        return list(self.additional_properties.keys())

    def __getitem__(self, key: str) -> Any:
        return self.additional_properties[key]

    def __setitem__(self, key: str, value: Any) -> None:
        self.additional_properties[key] = value

    def __delitem__(self, key: str) -> None:
        del self.additional_properties[key]

    def __contains__(self, key: str) -> bool:
        return key in self.additional_properties
