from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..models.data_class import DataClass
from ..types import UNSET, Unset
from typing import cast
import datetime

if TYPE_CHECKING:
  from ..models.page_hold_response_data_item_held_type_0 import PageHoldResponseDataItemHeldType0
  from ..models.subject_identifier_type_0 import SubjectIdentifierType0
  from ..models.subject_identifier_type_1 import SubjectIdentifierType1
  from ..models.subject_identifier_type_2 import SubjectIdentifierType2
  from ..models.subject_identifier_type_3 import SubjectIdentifierType3





T = TypeVar("T", bound="PageHoldResponseDataItem")



@_attrs_define
class PageHoldResponseDataItem:
    """ 
        Attributes:
            id (str):
            tenant_id (str):
            matter (str):
            subjects (list[SubjectIdentifierType0 | SubjectIdentifierType1 | SubjectIdentifierType2 |
                SubjectIdentifierType3]):
            call_ids (list[str]):
            classes (list[DataClass]):
            subject_keys (list[str]): The subjects as the store matches them.
            created_by (str):
            created_at (datetime.datetime):
            active (bool):
            reason (None | str | Unset):
            from_ (int | None | Unset):
            to (int | None | Unset):
            released_by (None | str | Unset):
            released_at (datetime.datetime | None | Unset):
            release_reason (None | str | Unset):
            held (None | PageHoldResponseDataItemHeldType0 | Unset): Items held now, by class; `null` for a class too large
                to count
                quickly. Only on `GET /compliance/holds/:id`.
     """

    id: str
    tenant_id: str
    matter: str
    subjects: list[SubjectIdentifierType0 | SubjectIdentifierType1 | SubjectIdentifierType2 | SubjectIdentifierType3]
    call_ids: list[str]
    classes: list[DataClass]
    subject_keys: list[str]
    created_by: str
    created_at: datetime.datetime
    active: bool
    reason: None | str | Unset = UNSET
    from_: int | None | Unset = UNSET
    to: int | None | Unset = UNSET
    released_by: None | str | Unset = UNSET
    released_at: datetime.datetime | None | Unset = UNSET
    release_reason: None | str | Unset = UNSET
    held: None | PageHoldResponseDataItemHeldType0 | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.page_hold_response_data_item_held_type_0 import PageHoldResponseDataItemHeldType0 # noqa: PLC0415
        from ..models.subject_identifier_type_0 import SubjectIdentifierType0 # noqa: PLC0415
        from ..models.subject_identifier_type_1 import SubjectIdentifierType1 # noqa: PLC0415
        from ..models.subject_identifier_type_2 import SubjectIdentifierType2 # noqa: PLC0415
        from ..models.subject_identifier_type_3 import SubjectIdentifierType3 # noqa: PLC0415
        id = self.id

        tenant_id = self.tenant_id

        matter = self.matter

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



        call_ids = self.call_ids



        classes = []
        for classes_item_data in self.classes:
            classes_item = classes_item_data.value
            classes.append(classes_item)



        subject_keys = self.subject_keys



        created_by = self.created_by

        created_at = self.created_at.isoformat()

        active = self.active

        reason: None | str | Unset
        if isinstance(self.reason, Unset):
            reason = UNSET
        else:
            reason = self.reason

        from_: int | None | Unset
        if isinstance(self.from_, Unset):
            from_ = UNSET
        else:
            from_ = self.from_

        to: int | None | Unset
        if isinstance(self.to, Unset):
            to = UNSET
        else:
            to = self.to

        released_by: None | str | Unset
        if isinstance(self.released_by, Unset):
            released_by = UNSET
        else:
            released_by = self.released_by

        released_at: None | str | Unset
        if isinstance(self.released_at, Unset):
            released_at = UNSET
        elif isinstance(self.released_at, datetime.datetime):
            released_at = self.released_at.isoformat()
        else:
            released_at = self.released_at

        release_reason: None | str | Unset
        if isinstance(self.release_reason, Unset):
            release_reason = UNSET
        else:
            release_reason = self.release_reason

        held: dict[str, Any] | None | Unset
        if isinstance(self.held, Unset):
            held = UNSET
        elif isinstance(self.held, PageHoldResponseDataItemHeldType0):
            held = self.held.to_dict()
        else:
            held = self.held


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "id": id,
            "tenant_id": tenant_id,
            "matter": matter,
            "subjects": subjects,
            "call_ids": call_ids,
            "classes": classes,
            "subject_keys": subject_keys,
            "created_by": created_by,
            "created_at": created_at,
            "active": active,
        })
        if reason is not UNSET:
            field_dict["reason"] = reason
        if from_ is not UNSET:
            field_dict["from"] = from_
        if to is not UNSET:
            field_dict["to"] = to
        if released_by is not UNSET:
            field_dict["released_by"] = released_by
        if released_at is not UNSET:
            field_dict["released_at"] = released_at
        if release_reason is not UNSET:
            field_dict["release_reason"] = release_reason
        if held is not UNSET:
            field_dict["held"] = held

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.page_hold_response_data_item_held_type_0 import PageHoldResponseDataItemHeldType0 # noqa: PLC0415
        from ..models.subject_identifier_type_0 import SubjectIdentifierType0 # noqa: PLC0415
        from ..models.subject_identifier_type_1 import SubjectIdentifierType1 # noqa: PLC0415
        from ..models.subject_identifier_type_2 import SubjectIdentifierType2 # noqa: PLC0415
        from ..models.subject_identifier_type_3 import SubjectIdentifierType3 # noqa: PLC0415
        d = dict(src_dict)
        id = d.pop("id")

        tenant_id = d.pop("tenant_id")

        matter = d.pop("matter")

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


        call_ids = cast(list[str], d.pop("call_ids"))


        classes = []
        _classes = d.pop("classes")
        for classes_item_data in (_classes):
            classes_item = DataClass(classes_item_data)



            classes.append(classes_item)


        subject_keys = cast(list[str], d.pop("subject_keys"))


        created_by = d.pop("created_by")

        created_at = datetime.datetime.fromisoformat(d.pop("created_at"))




        active = d.pop("active")

        def _parse_reason(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        reason = _parse_reason(d.pop("reason", UNSET))


        def _parse_from_(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        from_ = _parse_from_(d.pop("from", UNSET))


        def _parse_to(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        to = _parse_to(d.pop("to", UNSET))


        def _parse_released_by(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        released_by = _parse_released_by(d.pop("released_by", UNSET))


        def _parse_released_at(data: object) -> datetime.datetime | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, str):
                    raise TypeError()
                released_at_type_0 = datetime.datetime.fromisoformat(data)



                return released_at_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(datetime.datetime | None | Unset, data)

        released_at = _parse_released_at(d.pop("released_at", UNSET))


        def _parse_release_reason(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        release_reason = _parse_release_reason(d.pop("release_reason", UNSET))


        def _parse_held(data: object) -> None | PageHoldResponseDataItemHeldType0 | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, dict):
                    raise TypeError()
                held_type_0 = PageHoldResponseDataItemHeldType0.from_dict(data)



                return held_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(None | PageHoldResponseDataItemHeldType0 | Unset, data)

        held = _parse_held(d.pop("held", UNSET))


        page_hold_response_data_item = cls(
            id=id,
            tenant_id=tenant_id,
            matter=matter,
            subjects=subjects,
            call_ids=call_ids,
            classes=classes,
            subject_keys=subject_keys,
            created_by=created_by,
            created_at=created_at,
            active=active,
            reason=reason,
            from_=from_,
            to=to,
            released_by=released_by,
            released_at=released_at,
            release_reason=release_reason,
            held=held,
        )


        page_hold_response_data_item.additional_properties = d
        return page_hold_response_data_item

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
