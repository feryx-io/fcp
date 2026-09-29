from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast

if TYPE_CHECKING:
  from ..models.subject_identifier_type_0 import SubjectIdentifierType0
  from ..models.subject_identifier_type_1 import SubjectIdentifierType1
  from ..models.subject_identifier_type_2 import SubjectIdentifierType2
  from ..models.subject_identifier_type_3 import SubjectIdentifierType3





T = TypeVar("T", bound="FileSubjectRequest")



@_attrs_define
class FileSubjectRequest:
    """ 
        Attributes:
            kind (str): `export` or `erasure`.
            subjects (list[SubjectIdentifierType0 | SubjectIdentifierType1 | SubjectIdentifierType2 |
                SubjectIdentifierType3]):
            reference (None | str | Unset): The tenant's own reference: a ticket, a letter.
     """

    kind: str
    subjects: list[SubjectIdentifierType0 | SubjectIdentifierType1 | SubjectIdentifierType2 | SubjectIdentifierType3]
    reference: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.subject_identifier_type_0 import SubjectIdentifierType0 # noqa: PLC0415
        from ..models.subject_identifier_type_1 import SubjectIdentifierType1 # noqa: PLC0415
        from ..models.subject_identifier_type_2 import SubjectIdentifierType2 # noqa: PLC0415
        from ..models.subject_identifier_type_3 import SubjectIdentifierType3 # noqa: PLC0415
        kind = self.kind

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



        reference: None | str | Unset
        if isinstance(self.reference, Unset):
            reference = UNSET
        else:
            reference = self.reference


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "kind": kind,
            "subjects": subjects,
        })
        if reference is not UNSET:
            field_dict["reference"] = reference

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.subject_identifier_type_0 import SubjectIdentifierType0 # noqa: PLC0415
        from ..models.subject_identifier_type_1 import SubjectIdentifierType1 # noqa: PLC0415
        from ..models.subject_identifier_type_2 import SubjectIdentifierType2 # noqa: PLC0415
        from ..models.subject_identifier_type_3 import SubjectIdentifierType3 # noqa: PLC0415
        d = dict(src_dict)
        kind = d.pop("kind")

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


        def _parse_reference(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        reference = _parse_reference(d.pop("reference", UNSET))


        file_subject_request = cls(
            kind=kind,
            subjects=subjects,
            reference=reference,
        )


        file_subject_request.additional_properties = d
        return file_subject_request

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
