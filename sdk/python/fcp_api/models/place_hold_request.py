from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..models.data_class import DataClass
from ..types import UNSET, Unset
from typing import cast

if TYPE_CHECKING:
  from ..models.subject_identifier_type_0 import SubjectIdentifierType0
  from ..models.subject_identifier_type_1 import SubjectIdentifierType1
  from ..models.subject_identifier_type_2 import SubjectIdentifierType2
  from ..models.subject_identifier_type_3 import SubjectIdentifierType3





T = TypeVar("T", bound="PlaceHoldRequest")



@_attrs_define
class PlaceHoldRequest:
    """ 
        Attributes:
            matter (str):
            reason (None | str | Unset):
            from_ (int | None | Unset): Unix seconds; data from this instant.
            to (int | None | Unset): Unix seconds; data before this instant.
            subjects (list[SubjectIdentifierType0 | SubjectIdentifierType1 | SubjectIdentifierType2 |
                SubjectIdentifierType3] | Unset):
            call_ids (list[str] | Unset):
            classes (list[DataClass] | Unset):
     """

    matter: str
    reason: None | str | Unset = UNSET
    from_: int | None | Unset = UNSET
    to: int | None | Unset = UNSET
    subjects: list[SubjectIdentifierType0 | SubjectIdentifierType1 | SubjectIdentifierType2 | SubjectIdentifierType3] | Unset = UNSET
    call_ids: list[str] | Unset = UNSET
    classes: list[DataClass] | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.subject_identifier_type_0 import SubjectIdentifierType0 # noqa: PLC0415
        from ..models.subject_identifier_type_1 import SubjectIdentifierType1 # noqa: PLC0415
        from ..models.subject_identifier_type_2 import SubjectIdentifierType2 # noqa: PLC0415
        from ..models.subject_identifier_type_3 import SubjectIdentifierType3 # noqa: PLC0415
        matter = self.matter

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

        subjects: list[dict[str, Any]] | Unset = UNSET
        if not isinstance(self.subjects, Unset):
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



        call_ids: list[str] | Unset = UNSET
        if not isinstance(self.call_ids, Unset):
            call_ids = self.call_ids



        classes: list[str] | Unset = UNSET
        if not isinstance(self.classes, Unset):
            classes = []
            for classes_item_data in self.classes:
                classes_item = classes_item_data.value
                classes.append(classes_item)




        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "matter": matter,
        })
        if reason is not UNSET:
            field_dict["reason"] = reason
        if from_ is not UNSET:
            field_dict["from"] = from_
        if to is not UNSET:
            field_dict["to"] = to
        if subjects is not UNSET:
            field_dict["subjects"] = subjects
        if call_ids is not UNSET:
            field_dict["call_ids"] = call_ids
        if classes is not UNSET:
            field_dict["classes"] = classes

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.subject_identifier_type_0 import SubjectIdentifierType0 # noqa: PLC0415
        from ..models.subject_identifier_type_1 import SubjectIdentifierType1 # noqa: PLC0415
        from ..models.subject_identifier_type_2 import SubjectIdentifierType2 # noqa: PLC0415
        from ..models.subject_identifier_type_3 import SubjectIdentifierType3 # noqa: PLC0415
        d = dict(src_dict)
        matter = d.pop("matter")

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


        _subjects = d.pop("subjects", UNSET)
        subjects: list[SubjectIdentifierType0 | SubjectIdentifierType1 | SubjectIdentifierType2 | SubjectIdentifierType3] | Unset = UNSET
        if _subjects is not UNSET:
            subjects = []
            for subjects_item_data in _subjects:
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


        call_ids = cast(list[str], d.pop("call_ids", UNSET))


        _classes = d.pop("classes", UNSET)
        classes: list[DataClass] | Unset = UNSET
        if _classes is not UNSET:
            classes = []
            for classes_item_data in _classes:
                classes_item = DataClass(classes_item_data)



                classes.append(classes_item)


        place_hold_request = cls(
            matter=matter,
            reason=reason,
            from_=from_,
            to=to,
            subjects=subjects,
            call_ids=call_ids,
            classes=classes,
        )


        place_hold_request.additional_properties = d
        return place_hold_request

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
